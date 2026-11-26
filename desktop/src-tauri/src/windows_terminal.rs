use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, SAFEARRAY,
};
use windows::Win32::System::Ole::{
    SafeArrayDestroy, SafeArrayGetDim, SafeArrayGetElement, SafeArrayGetLBound,
    SafeArrayGetUBound,
};
use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_I4};
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation2, IUIAutomationCondition, IUIAutomationElement,
    IUIAutomationSelectionItemPattern, TreeScope_Descendants, UIA_ControlTypePropertyId,
    UIA_SelectionItemPatternId, UIA_TabItemControlTypeId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindowAsync,
};

const PROBE_INTERVAL: Duration = Duration::from_millis(40);
const TITLE_TIMEOUT: Duration = Duration::from_millis(1200);
const SELECTION_TIMEOUT: Duration = Duration::from_millis(500);

/// The accessibility provider identifies the tab independently of its mutable title.
#[derive(PartialEq, Eq)]
struct TabId {
    window: isize,
    runtime_id: Vec<i32>,
}

pub(crate) struct TabTitle {
    id: TabId,
    name: String,
}

#[derive(Default)]
pub(crate) struct MarkedTabResult {
    pub(crate) selected: bool,
    /// A title to restore when the transport changed it without recording the original.
    pub(crate) original_title: Option<String>,
}

struct Apartment;

impl Apartment {
    fn new() -> windows::core::Result<Self> {
        // SAFETY: each caller runs on a background worker and balances this
        // successful COM initialization on the same thread.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: this guard owns one successful COM initialization.
        unsafe { CoUninitialize() };
    }
}

struct Automation {
    automation: IUIAutomation2,
    tab_condition: IUIAutomationCondition,
    _apartment: Apartment,
}

impl Automation {
    fn new() -> windows::core::Result<Self> {
        let apartment = Apartment::new()?;
        let tab_control = VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                    vt: VT_I4,
                    Anonymous: VARIANT_0_0_0 {
                        lVal: UIA_TabItemControlTypeId.0,
                    },
                    ..Default::default()
                }),
            },
        };
        // SAFETY: COM is initialized on this thread, the CLSID is built in to
        // Windows, and the property filter holds a scalar I4 with no allocation.
        unsafe {
            let automation: IUIAutomation2 =
                CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
            automation.SetConnectionTimeout(700)?;
            automation.SetTransactionTimeout(700)?;
            let tab_condition = automation.CreatePropertyCondition(UIA_ControlTypePropertyId, &tab_control)?;
            Ok(Self { automation, tab_condition, _apartment: apartment })
        }
    }

    fn tabs(&self) -> Vec<(TabTitle, IUIAutomationElement)> {
        // SAFETY: HWNDs come from the live Windows Terminal window enumeration;
        // UI Automation interfaces are used only on their initializing thread.
        unsafe {
            crate::terminal::windows_terminal_windows()
                .into_iter()
                .flat_map(|window| {
                    let tabs = self
                        .automation
                        .ElementFromHandle(HWND(window as *mut c_void))
                        .and_then(|root| root.FindAll(TreeScope_Descendants, &self.tab_condition));
                    let Ok(tabs) = tabs else { return Vec::new() };
                    (0..tabs.Length().unwrap_or_default())
                        .filter_map(|index| {
                            let element = tabs.GetElement(index).ok()?;
                            let name = element.CurrentName().ok()?.to_string();
                            let runtime_id = runtime_id(&element)?;
                            let id = TabId { window, runtime_id };
                            Some((TabTitle { id, name }, element))
                        })
                        .collect()
                })
                .collect()
        }
    }
}

struct RuntimeIdArray(*mut SAFEARRAY);

impl Drop for RuntimeIdArray {
    fn drop(&mut self) {
        // SAFETY: GetRuntimeId transfers ownership of the non-null array.
        unsafe { let _ = SafeArrayDestroy(self.0); }
    }
}

fn runtime_id(element: &IUIAutomationElement) -> Option<Vec<i32>> {
    // SAFETY: GetRuntimeId returns a caller-owned SAFEARRAY of 32-bit integers.
    // All accesses are limited to its reported bounds and written into an i32.
    unsafe {
        let pointer = element.GetRuntimeId().ok()?;
        if pointer.is_null() {
            return None;
        }
        let array = RuntimeIdArray(pointer);
        if SafeArrayGetDim(array.0) != 1 {
            return None;
        }
        let first = SafeArrayGetLBound(array.0, 1).ok()?;
        let last = SafeArrayGetUBound(array.0, 1).ok()?;
        if !(0..32).contains(&last.checked_sub(first)?) {
            return None;
        }
        (first..=last)
            .map(|index| {
                let mut value = 0i32;
                SafeArrayGetElement(array.0, &index, (&mut value as *mut i32).cast()).ok()?;
                Some(value)
            })
            .collect()
    }
}

/// Capture before writing a transient title. Call from a blocking worker.
pub(crate) fn snapshot_tab_titles() -> Vec<TabTitle> {
    Automation::new().map_or_else(
        |_| Vec::new(),
        |automation| automation.tabs().into_iter().map(|(title, _)| title).collect(),
    )
}

/// Select the unique tab that exposes the caller's transient title. A raised
/// window is successful only when this exact TabItem reports itself selected.
/// Call from a blocking worker.
pub(crate) fn activate_marked_tab(marker: &str, before: &[TabTitle]) -> MarkedTabResult {
    let Ok(automation) = Automation::new() else {
        return MarkedTabResult::default();
    };
    let start = Instant::now();
    loop {
        let mut matching = automation
            .tabs()
            .into_iter()
            .filter(|(title, _)| title.name == marker);
        if let Some((title, element)) = matching.next() {
            if matching.next().is_some() {
                return MarkedTabResult::default();
            }
            let selected = select_tab(&title.id, &element);
            // SAFETY: the matched element is still on this thread's live UIA
            // connection. A title set by the session while switching wins.
            let marker_remains = unsafe {
                element.CurrentName().is_ok_and(|name| name.to_string() == marker)
            };
            let original_title = if marker_remains {
                before.iter().find(|old| old.id == title.id).map(|old| old.name.clone())
            } else {
                None
            };
            return MarkedTabResult {
                selected,
                original_title,
            };
        }
        if start.elapsed() >= TITLE_TIMEOUT {
            return MarkedTabResult::default();
        }
        thread::sleep(PROBE_INTERVAL);
    }
}

fn select_tab(tab: &TabId, element: &IUIAutomationElement) -> bool {
    let window = HWND(tab.window as *mut c_void);
    // SAFETY: the tab is matched uniquely through its live accessibility
    // provider before issuing a selection or foreground request.
    unsafe {
        let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
        else {
            return false;
        };
        if !pattern.CurrentIsSelected().is_ok_and(|selected| selected.as_bool())
            && pattern.Select().is_err()
        {
            return false;
        }
        if IsIconic(window).as_bool() {
            let _ = ShowWindowAsync(window, SW_RESTORE);
        }
        let _ = SetForegroundWindow(window);

        let start = Instant::now();
        loop {
            if GetForegroundWindow() == window
                && pattern.CurrentIsSelected().is_ok_and(|selected| selected.as_bool())
                && runtime_id(element).is_some_and(|id| id == tab.runtime_id)
            {
                return true;
            }
            if start.elapsed() >= SELECTION_TIMEOUT {
                return false;
            }
            thread::sleep(PROBE_INTERVAL);
        }
    }
}
