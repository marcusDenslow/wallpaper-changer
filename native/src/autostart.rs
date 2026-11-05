#[cfg(windows)]
mod imp {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY,
        RRF_RT_REG_SZ,
    };

    const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const APPROVED_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run");
    const VALUE_NAME: PCWSTR = w!("Wallpaper Switcher");

    pub fn is_enabled() -> bool {
        let registered = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE_NAME, RRF_RT_REG_SZ, None, None, None)
        } == ERROR_SUCCESS;
        if !registered {
            return false;
        }

        let mut flags = [0u8; 12];
        let mut len = flags.len() as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                APPROVED_KEY,
                VALUE_NAME,
                RRF_RT_REG_BINARY,
                None,
                Some(flags.as_mut_ptr().cast()),
                Some(&mut len),
            )
        };
        status != ERROR_SUCCESS || flags[0] & 1 == 0
    }

    pub fn enable() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let command = format!("\"{}\" --autostart", exe.display());
        let data: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                VALUE_NAME,
                REG_SZ.0,
                Some(data.as_ptr().cast()),
                (data.len() * 2) as u32,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(format!("Windows didn't accept the start-up entry (error {})", status.0));
        }
        unsafe {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, APPROVED_KEY, VALUE_NAME);
        }
        Ok(())
    }

    pub fn disable() -> Result<(), String> {
        unsafe {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE_NAME);
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, APPROVED_KEY, VALUE_NAME);
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn is_enabled() -> bool {
        false
    }

    pub fn enable() -> Result<(), String> {
        Err("Start with Windows only works on Windows".into())
    }

    pub fn disable() -> Result<(), String> {
        Ok(())
    }
}

pub use imp::{disable, enable, is_enabled};

pub fn set(enabled: bool) -> Result<(), String> {
    if enabled {
        enable()
    } else {
        disable()
    }
}
