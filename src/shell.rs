use std::path::Path;
use windows::Win32::UI::Shell::{
    ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_COMMAND, AssocQueryStringW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

/// Opens a text file in its associated app, or in Notepad when it has none (as `.toml` often does).
pub fn open_text_file(path: &Path) -> Result<(), String> {
    let file = HSTRING::from(path.as_os_str());
    // Without this check, Windows shows its "How do you want to open this file?" dialog and still reports success.
    if has_open_command(path) && execute(&file, PCWSTR::null()) {
        return Ok(());
    }
    let parameters = HSTRING::from(format!("\"{}\"", path.display()));
    if execute(&HSTRING::from("notepad.exe"), PCWSTR(parameters.as_ptr())) {
        Ok(())
    } else {
        Err(format!("Could not open {}", path.display()))
    }
}

pub fn open_folder(path: &Path) -> Result<(), String> {
    if execute(&HSTRING::from(path.as_os_str()), PCWSTR::null()) {
        Ok(())
    } else {
        Err(format!("Could not open {}", path.display()))
    }
}

/// Returns true when Windows has an app registered to open files like `path`.
fn has_open_command(path: &Path) -> bool {
    let Some(extension) = path.extension() else {
        return false;
    };
    let mut extension_with_dot = std::ffi::OsString::from(".");
    extension_with_dot.push(extension);
    let extension = HSTRING::from(extension_with_dot.as_os_str());
    let mut length = 0u32;
    // SAFETY: `extension` is a null-terminated string, and only the length of the result is requested.
    let result = unsafe {
        AssocQueryStringW(
            ASSOCF_INIT_IGNOREUNKNOWN,
            ASSOCSTR_COMMAND,
            &extension,
            w!("open"),
            None,
            &mut length,
        )
    };
    result.is_ok()
}

fn execute(file: &HSTRING, parameters: PCWSTR) -> bool {
    // SAFETY: `file` and `parameters` are null-terminated strings, or null, that outlive the call.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            file,
            parameters,
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean success, as documented for ShellExecuteW.
    result.0.addr() > 32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_file_type_has_open_command() {
        let path = Path::new(r"C:\Windows\notepad.exe");

        assert!(has_open_command(path));
    }

    #[test]
    fn unregistered_file_type_has_no_open_command() {
        let path = Path::new(r"C:\Apps\settings.vrc-shot-relay-unregistered");

        assert!(!has_open_command(path));
    }

    #[test]
    fn file_without_extension_has_no_open_command() {
        let path = Path::new(r"C:\Apps\settings");

        assert!(!has_open_command(path));
    }
}
