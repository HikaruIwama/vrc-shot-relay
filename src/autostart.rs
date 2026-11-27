use std::path::Path;
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::HSTRING;

/// Command line that the Run key starts at sign-in.
pub fn command_line(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// A registry value under HKEY_CURRENT_USER that starts a program at sign-in.
pub struct RunKey {
    subkey: HSTRING,
    value_name: HSTRING,
}

impl RunKey {
    pub fn current_user() -> Self {
        Self::new(
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            "VRC Shot Relay",
        )
    }

    fn new(subkey: &str, value_name: &str) -> Self {
        Self {
            subkey: HSTRING::from(subkey),
            value_name: HSTRING::from(value_name),
        }
    }

    /// Returns the registered command line, or `None` when nothing is registered.
    pub fn registered_command(&self) -> Result<Option<String>, String> {
        let mut size = 0u32;
        // SAFETY: The names are valid null-terminated strings, and only the size is requested.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &self.subkey,
                &self.value_name,
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        status
            .ok()
            .map_err(|error| format!("Could not read startup setting: {error}"))?;
        let length = usize::try_from(size).map_err(|error| error.to_string())? / 2;
        let mut buffer = vec![0u16; length];
        // SAFETY: The buffer holds `size` bytes, as `RegGetValueW` reported for this value.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &self.subkey,
                &self.value_name,
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        status
            .ok()
            .map_err(|error| format!("Could not read startup setting: {error}"))?;
        let end = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        String::from_utf16(&buffer[..end])
            .map(Some)
            .map_err(|error| format!("Could not read startup setting: {error}"))
    }

    /// Returns true only when the registration starts `exe`, so a moved program reads as off.
    pub fn is_enabled(&self, exe: &Path) -> Result<bool, String> {
        Ok(self.registered_command()? == Some(command_line(exe)))
    }

    pub fn set_enabled(&self, exe: &Path, enabled: bool) -> Result<(), String> {
        if enabled {
            self.register(exe)
        } else {
            self.unregister()
        }
    }

    fn register(&self, exe: &Path) -> Result<(), String> {
        let data: Vec<u16> = command_line(exe).encode_utf16().chain(Some(0)).collect();
        let size = u32::try_from(data.len() * 2).map_err(|error| error.to_string())?;
        // SAFETY: `data` is a null-terminated UTF-16 string of `size` bytes that outlives the call.
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &self.subkey,
                &self.value_name,
                REG_SZ.0,
                Some(data.as_ptr().cast()),
                size,
            )
        }
        .ok()
        .map_err(|error| format!("Could not enable start with Windows: {error}"))
    }

    fn unregister(&self) -> Result<(), String> {
        // SAFETY: The names are valid null-terminated strings.
        let status =
            unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &self.subkey, &self.value_name) };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        status
            .ok()
            .map_err(|error| format!("Could not disable start with Windows: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteTreeW};

    struct TestKey {
        subkey: String,
        key: RunKey,
    }

    impl TestKey {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let subkey = format!(r"Software\vrc-shot-relay-test-{stamp}");
            let key = RunKey::new(&subkey, "VRC Shot Relay");
            Self { subkey, key }
        }
    }

    impl Drop for TestKey {
        fn drop(&mut self) {
            // SAFETY: The handle is a predefined root key and the subkey is a valid null-terminated string.
            unsafe {
                let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(self.subkey.as_str()));
            }
        }
    }

    #[test]
    fn command_line_quotes_path_with_spaces() {
        let exe = Path::new(r"C:\Program Files\VRC Shot Relay\vrc-shot-relay.exe");

        let result = command_line(exe);

        assert_eq!(
            result,
            r#""C:\Program Files\VRC Shot Relay\vrc-shot-relay.exe""#
        );
    }

    #[test]
    fn nothing_registered_reads_as_off() {
        let test = TestKey::new();
        let exe = Path::new(r"C:\Apps\vrc-shot-relay.exe");

        let enabled = test.key.is_enabled(exe).unwrap();

        assert!(!enabled);
        assert_eq!(test.key.registered_command().unwrap(), None);
    }

    #[test]
    fn enabling_registers_quoted_program_path() {
        let test = TestKey::new();
        let exe = Path::new(r"C:\Program Files\VRC Shot Relay\vrc-shot-relay.exe");

        test.key.set_enabled(exe, true).unwrap();

        assert!(test.key.is_enabled(exe).unwrap());
        assert_eq!(
            test.key.registered_command().unwrap(),
            Some(r#""C:\Program Files\VRC Shot Relay\vrc-shot-relay.exe""#.into())
        );
    }

    #[test]
    fn disabling_removes_registration() {
        let test = TestKey::new();
        let exe = Path::new(r"C:\Apps\vrc-shot-relay.exe");
        test.key.set_enabled(exe, true).unwrap();

        test.key.set_enabled(exe, false).unwrap();

        assert!(!test.key.is_enabled(exe).unwrap());
        assert_eq!(test.key.registered_command().unwrap(), None);
    }

    #[test]
    fn disabling_without_registration_succeeds() {
        let test = TestKey::new();
        let exe = Path::new(r"C:\Apps\vrc-shot-relay.exe");

        let result = test.key.set_enabled(exe, false);

        assert!(result.is_ok());
    }

    #[test]
    fn registration_for_moved_program_reads_as_off() {
        let test = TestKey::new();
        test.key
            .set_enabled(Path::new(r"C:\Old\vrc-shot-relay.exe"), true)
            .unwrap();

        let enabled = test
            .key
            .is_enabled(Path::new(r"D:\New\vrc-shot-relay.exe"))
            .unwrap();

        assert!(!enabled);
    }
}
