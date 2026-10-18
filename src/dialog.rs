use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::{PCWSTR, w};

pub fn show_startup_error(message: &str) {
    let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    // SAFETY: The null-terminated message buffer remains valid until MessageBoxW returns.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            w!("VRC Shot Relay - Could not start"),
            MB_OK | MB_ICONERROR,
        );
    }
}
