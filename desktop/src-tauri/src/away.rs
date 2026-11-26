//! Windows equivalents of the lock, screensaver, and display sleep signals.

use std::ffi::c_void;
use std::mem::size_of;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use codeisland_runtime::Runtime;
use windows::Win32::Foundation::{HANDLE, WIN32_ERROR};
use windows::Win32::System::Power::{
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, HPOWERNOTIFY, POWERBROADCAST_SETTING,
    PowerSettingRegisterNotification, PowerSettingUnregisterNotification,
};
use windows::Win32::System::RemoteDesktop::{
    WTS_CURRENT_SESSION, WTS_SESSIONSTATE_LOCK, WTSFreeMemory, WTSINFOEXW,
    WTSQuerySessionInformationW, WTSSessionInfoEx,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::SystemServices::GUID_SESSION_DISPLAY_STATUS;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    DEVICE_NOTIFY_CALLBACK, PBT_POWERSETTINGCHANGE, SPI_GETSCREENSAVERRUNNING,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{BOOL, PWSTR};

static DISPLAY_OFF: AtomicBool = AtomicBool::new(false);

pub fn spawn(runtime: &Arc<Runtime>) {
    let runtime = Arc::downgrade(runtime);
    std::thread::spawn(move || {
        let mut subscription = DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(display_changed),
            Context: std::ptr::null_mut(),
        };
        let mut registration = std::ptr::null_mut();
        // SAFETY: subscription remains in place until the registration is
        // released. The callback only writes the process-lifetime atomic.
        let registered = unsafe {
            PowerSettingRegisterNotification(
                &GUID_SESSION_DISPLAY_STATUS,
                DEVICE_NOTIFY_CALLBACK,
                HANDLE((&mut subscription as *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS).cast()),
                &mut registration,
            )
        } == WIN32_ERROR(0);
        while let Some(runtime) = runtime.upgrade() {
            if let Some(seconds) = idle_seconds() {
                runtime.set_idle_seconds(seconds);
            }
            runtime.set_display_asleep(DISPLAY_OFF.load(Ordering::Relaxed));
            runtime.set_away(is_away_now());
            drop(runtime);
            std::thread::sleep(Duration::from_millis(500));
        }
        if registered {
            // SAFETY: this is the registration returned by the call above.
            unsafe {
                let _ = PowerSettingUnregisterNotification(HPOWERNOTIFY(registration as isize));
            }
        }
    });
}

pub(crate) fn is_away_now() -> bool {
    session_locked() || screensaver_running()
}

fn idle_seconds() -> Option<u64> {
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: these APIs only read session input timing. Both timestamps are
    // 32-bit tick counts, so subtraction must allow for rollover.
    unsafe {
        GetLastInputInfo(&mut info)
            .as_bool()
            .then(|| u64::from(GetTickCount().wrapping_sub(info.dwTime)) / 1_000)
    }
}

unsafe extern "system" fn display_changed(
    _context: *const c_void,
    event: u32,
    setting: *const c_void,
) -> u32 {
    if event != PBT_POWERSETTINGCHANGE || setting.is_null() {
        return 0;
    }
    // SAFETY: Windows supplies a POWERBROADCAST_SETTING for this callback.
    let setting = unsafe { &*setting.cast::<POWERBROADCAST_SETTING>() };
    if setting.PowerSetting == GUID_SESSION_DISPLAY_STATUS
        && setting.DataLength >= size_of::<u32>() as u32
    {
        // SAFETY: the OS-provided trailing data contains a DWORD.
        let state = unsafe { setting.Data.as_ptr().cast::<u32>().read_unaligned() };
        DISPLAY_OFF.store(state == 0, Ordering::Relaxed);
    }
    0
}

fn session_locked() -> bool {
    let mut buffer = PWSTR::null();
    let mut length = 0;
    // SAFETY: the query owns and initializes the buffer on success. Read the
    // level-1 member only after checking the returned length and level.
    unsafe {
        if WTSQuerySessionInformationW(
            None,
            WTS_CURRENT_SESSION,
            WTSSessionInfoEx,
            &mut buffer,
            &mut length,
        )
        .is_err()
        {
            return false;
        }
        let locked = !buffer.is_null() && length as usize >= size_of::<WTSINFOEXW>() && {
            let info = &*buffer.0.cast::<WTSINFOEXW>();
            info.Level == 1
                && info.Data.WTSInfoExLevel1.SessionFlags as u32 == WTS_SESSIONSTATE_LOCK
        };
        if !buffer.is_null() {
            WTSFreeMemory(buffer.0.cast());
        }
        locked
    }
}

fn screensaver_running() -> bool {
    let mut running = BOOL(0);
    // SAFETY: this query writes one BOOL into our stack value.
    unsafe {
        SystemParametersInfoW(
            SPI_GETSCREENSAVERRUNNING,
            0,
            Some((&mut running as *mut BOOL).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && running.as_bool()
    }
}
