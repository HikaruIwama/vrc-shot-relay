use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use vrc_shot_relay::engine::{Command, UiEvent};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_ERROR, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const TRAY_MESSAGE: u32 = WM_APP + 1;
const PAUSE_ID: usize = 1;
const EXIT_ID: usize = 2;
static STATE: OnceLock<Mutex<TrayState>> = OnceLock::new();

struct TrayState {
    sender: Option<Sender<Command>>,
    events: Receiver<UiEvent>,
    status: String,
    paused: bool,
}

pub fn run(
    sender: Option<Sender<Command>>,
    events: Receiver<UiEvent>,
    status: String,
) -> Result<(), String> {
    STATE
        .set(Mutex::new(TrayState {
            sender,
            events,
            status,
            paused: false,
        }))
        .map_err(|_| "Tray already running".to_string())?;
    unsafe {
        let module = GetModuleHandleW(None).map_err(|error| error.to_string())?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: windows::Win32::Foundation::HINSTANCE(module.0),
            lpszClassName: w!("VrcShotRelayWindow"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err("Could not register tray window".into());
        }
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("VrcShotRelayWindow"),
            w!("VRC Shot Relay"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(class.hInstance),
            None,
        )
        .map_err(|error| format!("Could not create tray window: {error}"))?;
        let mut icon = icon_data(hwnd);
        icon.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        icon.uCallbackMessage = TRAY_MESSAGE;
        icon.hIcon = LoadIconW(None, IDI_APPLICATION).map_err(|error| error.to_string())?;
        set_text(&mut icon.szTip, "VRC Shot Relay");
        if !Shell_NotifyIconW(NIM_ADD, &icon).as_bool() {
            return Err("Could not add tray icon".into());
        }
        SetTimer(Some(hwnd), 1, 500, None);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        TRAY_MESSAGE
            if matches!(
                lparam.0 as u32,
                WM_RBUTTONUP | WM_LBUTTONUP | WM_CONTEXTMENU
            ) =>
        {
            show_menu(hwnd);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 & 0xffff;
            if let Some(state) = STATE.get() {
                let mut state = state.lock().unwrap();
                match id {
                    PAUSE_ID => {
                        state.paused = !state.paused;
                        if let Some(sender) = &state.sender {
                            let _ = sender.send(Command::Pause(state.paused));
                        }
                        state.status = if state.paused {
                            "Paused"
                        } else {
                            "Watching photos"
                        }
                        .into();
                        update_tip(hwnd, &state.status);
                    }
                    EXIT_ID => {
                        if let Some(sender) = &state.sender {
                            let _ = sender.send(Command::Stop);
                        }
                        let _ = unsafe { DestroyWindow(hwnd) };
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if let Some(state) = STATE.get() {
                let mut state = state.lock().unwrap();
                while let Ok(event) = state.events.try_recv() {
                    match event {
                        UiEvent::Status(status) => {
                            state.status = status;
                            update_tip(hwnd, &state.status);
                        }
                        UiEvent::Error(error) => {
                            state.status = "Error - see relay.log".into();
                            update_tip(hwnd, &state.status);
                            show_error(hwnd, &error);
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let icon = icon_data(hwnd);
            let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &icon) };
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    }
}

fn set_text(buffer: &mut [u16], text: &str) {
    buffer.fill(0);
    let length = buffer.len().saturating_sub(1);
    for (slot, value) in buffer.iter_mut().take(length).zip(text.encode_utf16()) {
        *slot = value;
    }
}

fn update_tip(hwnd: HWND, status: &str) {
    let mut icon = icon_data(hwnd);
    icon.uFlags = NIF_TIP;
    set_text(&mut icon.szTip, &format!("VRC Shot Relay - {status}"));
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &icon);
    }
}

fn show_error(hwnd: HWND, error: &str) {
    let mut icon = icon_data(hwnd);
    icon.uFlags = NIF_INFO;
    icon.dwInfoFlags = NIIF_ERROR;
    set_text(&mut icon.szInfoTitle, "VRC Shot Relay");
    set_text(&mut icon.szInfo, error);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &icon);
    }
}

fn show_menu(hwnd: HWND) {
    let Some(state) = STATE.get() else { return };
    let (status, can_pause, paused) = {
        let state = state.lock().unwrap();
        (state.status.clone(), state.sender.is_some(), state.paused)
    };
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let status: Vec<u16> = status.encode_utf16().chain(Some(0)).collect();
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, PCWSTR(status.as_ptr()));
        let flags = if can_pause {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        };
        let _ = AppendMenuW(
            menu,
            flags,
            PAUSE_ID,
            if paused { w!("Resume") } else { w!("Pause") },
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, EXIT_ID, w!("Exit"));
        let mut point = POINT::default();
        if GetCursorPos(&mut point).is_ok() {
            let _ = SetForegroundWindow(hwnd);
            let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, point.x, point.y, None, hwnd, None);
        }
        let _ = DestroyMenu(menu);
    }
}
