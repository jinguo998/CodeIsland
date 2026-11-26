//! Windows wake and network transitions used by the SSH reconnect policy.

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{FreeLibrary, HANDLE, WIN32_ERROR};
use windows::Win32::NetworkManagement::IpHelper::PNETWORK_CONNECTIVITY_HINT_CHANGE_CALLBACK;
use windows::Win32::Networking::WinSock::{
    NL_NETWORK_CONNECTIVITY_HINT, NetworkConnectivityLevelHintHidden as NETWORK_HIDDEN,
    NetworkConnectivityLevelHintNone as NETWORK_NONE,
    NetworkConnectivityLevelHintUnknown as NETWORK_UNKNOWN,
};
use windows::Win32::System::Power::{
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, POWERBROADCAST_SETTING,
    PowerRegisterSuspendResumeNotification, PowerSettingRegisterNotification,
};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::Win32::System::SystemServices::GUID_SESSION_DISPLAY_STATUS;
use windows::Win32::UI::WindowsAndMessaging::{
    DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_POWERSETTINGCHANGE,
};

use windows::core::{s, w};

use crate::remote_connections::RecoverySignals;

static RECOVERY: OnceLock<RecoverySignals> = OnceLock::new();
static DISPLAY_OFF: AtomicBool = AtomicBool::new(false);

/// The single-instance app keeps these callbacks until process exit.
pub(crate) fn register(signals: RecoverySignals) {
    if RECOVERY.set(signals).is_err() {
        return;
    }
    let wake = callback_parameters(system_power_changed);
    let display = callback_parameters(display_power_changed);
    let mut wake_registration = std::ptr::null_mut();
    let mut display_registration = std::ptr::null_mut();

    // SAFETY: the callback parameters and watch sender live until process
    // exit. The callback writes to a thread-safe watch channel.
    unsafe {
        let wake_result = PowerRegisterSuspendResumeNotification(
            DEVICE_NOTIFY_CALLBACK, wake, &mut wake_registration,
        );
        let display_result = PowerSettingRegisterNotification(
            &GUID_SESSION_DISPLAY_STATUS, DEVICE_NOTIFY_CALLBACK, display, &mut display_registration,
        );
        if wake_result != WIN32_ERROR(0) || display_result != WIN32_ERROR(0) {
            tracing::debug!(?wake_result, ?display_result, "SSH wake monitoring unavailable");
        }
    }
    register_network();
}

fn register_network() {
    type NotifyNetwork = unsafe extern "system" fn(
        PNETWORK_CONNECTIVITY_HINT_CHANGE_CALLBACK, *const c_void, bool, *mut HANDLE,
    ) -> WIN32_ERROR;

    // SAFETY: the DLL comes from System32, and the proc signature matches
    // the Windows header. Keep the library loaded after registration.
    unsafe {
        let Ok(library) = LoadLibraryExW(w!("iphlpapi.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) else {
            return;
        };
        if let Some(proc) = GetProcAddress(library, s!("NotifyNetworkConnectivityHintChange")) {
            let notify = std::mem::transmute::<_, NotifyNetwork>(proc);
            let mut registration = HANDLE::default();
            let result = notify(Some(network_changed), std::ptr::null(), true, &mut registration);
            if result == WIN32_ERROR(0) {
                return;
            }
            tracing::debug!(?result, "SSH network monitoring unavailable");
        }
        let _ = FreeLibrary(library);
    }
}

type PowerCallback = unsafe extern "system" fn(*const c_void, u32, *const c_void) -> u32;

fn callback_parameters(callback: PowerCallback) -> HANDLE {
    let parameters = Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
        Callback: Some(callback),
        Context: std::ptr::null_mut(),
    }));
    HANDLE((parameters as *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS).cast())
}

unsafe extern "system" fn system_power_changed(
    _context: *const c_void,
    event: u32,
    _setting: *const c_void,
) -> u32 {
    if event == PBT_APMRESUMEAUTOMATIC && let Some(signals) = RECOVERY.get() {
        signals.wake();
    }
    0
}

unsafe extern "system" fn display_power_changed(
    _context: *const c_void,
    event: u32,
    setting: *const c_void,
) -> u32 {
    if event != PBT_POWERSETTINGCHANGE || setting.is_null() {
        return 0;
    }
    // SAFETY: the OS supplies the power setting for this notification.
    let setting = unsafe { &*setting.cast::<POWERBROADCAST_SETTING>() };
    if setting.PowerSetting == GUID_SESSION_DISPLAY_STATUS
        && setting.DataLength >= size_of::<u32>() as u32
    {
        // SAFETY: the OS supplied a DWORD for the display state.
        let state = unsafe { setting.Data.as_ptr().cast::<u32>().read_unaligned() };
        if state == 0 {
            DISPLAY_OFF.store(true, Ordering::Relaxed);
        } else if state == 1 && DISPLAY_OFF.swap(false, Ordering::Relaxed)
            && let Some(signals) = RECOVERY.get()
        {
            signals.wake();
        }
    }
    0
}

unsafe extern "system" fn network_changed(
    _context: *const c_void,
    hint: NL_NETWORK_CONNECTIVITY_HINT,
) {
    let online = match hint.ConnectivityLevel {
        NETWORK_UNKNOWN | NETWORK_HIDDEN => None,
        NETWORK_NONE => Some(false),
        _ => Some(true),
    };
    if let Some(signals) = RECOVERY.get() {
        signals.network(online);
    }
}
