use crate::image::Result;
use crate::winhook::WinHook;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::{Controls::*, Input::KeyboardAndMouse::*};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: u32,
    pub key: u32,
}

pub const DEFAULTS: [Hotkey; 3] = [
    Hotkey {
        modifiers: MOD_CONTROL | MOD_SHIFT,
        key: 0x53,
    },
    Hotkey {
        modifiers: MOD_CONTROL | MOD_SHIFT,
        key: 0x57,
    },
    Hotkey {
        modifiers: MOD_CONTROL | MOD_SHIFT,
        key: 0x46,
    },
];

impl Hotkey {
    pub fn encode(self) -> String {
        format!("{}:{}", self.modifiers, self.key)
    }
    pub fn parse(value: &str) -> Option<Self> {
        let (modifiers, key) = value.split_once(':')?;
        let result = Self {
            modifiers: modifiers.parse().ok()?,
            key: key.parse().ok()?,
        };
        (result.modifiers & !(MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_WIN) == 0
            && result.key <= 254)
            .then_some(result)
    }
    pub fn control_value(self) -> usize {
        let mut flags = 0;
        if self.modifiers & MOD_CONTROL != 0 {
            flags |= HOTKEYF_CONTROL;
        }
        if self.modifiers & MOD_SHIFT != 0 {
            flags |= HOTKEYF_SHIFT;
        }
        if self.modifiers & MOD_ALT != 0 {
            flags |= HOTKEYF_ALT;
        }
        if matches!(
            self.key as u16,
            VK_LEFT
                | VK_RIGHT
                | VK_UP
                | VK_DOWN
                | VK_HOME
                | VK_END
                | VK_PRIOR
                | VK_NEXT
                | VK_INSERT
                | VK_DELETE
                | VK_SNAPSHOT
                | VK_DIVIDE
        ) {
            flags |= HOTKEYF_EXT;
        }
        self.key as usize | ((flags as usize) << 8)
    }
    pub fn from_control(value: usize) -> Self {
        let flags = (value >> 8) as u32;
        let mut modifiers = 0;
        if flags & HOTKEYF_CONTROL != 0 {
            modifiers |= MOD_CONTROL;
        }
        if flags & HOTKEYF_SHIFT != 0 {
            modifiers |= MOD_SHIFT;
        }
        if flags & HOTKEYF_ALT != 0 {
            modifiers |= MOD_ALT;
        }
        Self {
            modifiers,
            key: (value & 255) as u32,
        }
    }
    pub fn display(self) -> String {
        if self.key == 0 {
            return "Disabled".into();
        }
        let mut result = String::new();
        for (flag, text) in [
            (MOD_WIN, "Win+"),
            (MOD_CONTROL, "Ctrl+"),
            (MOD_ALT, "Alt+"),
            (MOD_SHIFT, "Shift+"),
        ] {
            if self.modifiers & flag != 0 {
                result.push_str(text);
            }
        }
        let key = match self.key {
            0x30..=0x39 | 0x41..=0x5a => char::from_u32(self.key).unwrap_or('?').to_string(),
            0x70..=0x87 => format!("F{}", self.key - 0x70 + 1),
            0x2c => "PrtSc".into(),
            0x20 => "Space".into(),
            0x1b => "Esc".into(),
            0x0d => "Enter".into(),
            0x09 => "Tab".into(),
            0x08 => "Backspace".into(),
            0x2d => "Insert".into(),
            0x2e => "Delete".into(),
            _ => unsafe {
                let scan = MapVirtualKeyW(self.key, MAPVK_VK_TO_VSC_EX);
                let code = ((scan & 255) << 16) | if scan & 0xff00 != 0 { 1 << 24 } else { 0 };
                let mut name = [0u16; 80];
                let n = GetKeyNameTextW(code as i32, name.as_mut_ptr(), name.len() as i32);
                if n > 0 {
                    String::from_utf16_lossy(&name[..n as usize])
                } else {
                    format!("Key {}", self.key)
                }
            },
        };
        result.push_str(&key);
        result
    }
}

fn validate(keys: &[Hotkey; 3]) -> Result<()> {
    for (i, key) in keys.iter().enumerate() {
        if key.key == 0 {
            continue;
        }
        if key.key == VK_F12 as u32 {
            return Err("Windows reserves F12 for the debugger. Choose another key.".into());
        }
        if keys[..i].contains(key) {
            return Err(format!(
                "{} is assigned to more than one capture mode.",
                key.display()
            ));
        }
        if key.modifiers == 0 && !(0x70..=0x87).contains(&key.key) && key.key != 0x2c {
            return Err("Use Win, Ctrl, Alt, or Shift with a key. Function keys and Print Screen can be used alone.".into());
        }
        if matches!(
            key.key as u16,
            VK_CONTROL | VK_SHIFT | VK_MENU | VK_LWIN | VK_RWIN
        ) {
            return Err("Add a non-modifier key to the shortcut.".into());
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Binding {
    id: i32,
    key: Hotkey,
    hooked: bool,
}

pub struct Hotkeys {
    bindings: [Option<Binding>; 3],
    next_id: i32,
    hook: Option<WinHook>,
    block_snipping: bool,
}
impl Hotkeys {
    pub fn new() -> Self {
        Self {
            bindings: [None; 3],
            next_id: 20,
            hook: None,
            block_snipping: false,
        }
    }
    unsafe fn acquire(&mut self, owner: HWND, key: Hotkey) -> Result<Binding> {
        // Reuse IDs only after they are unregistered; keep all IDs below 0xC000.
        for _ in 0..100 {
            self.next_id = if self.next_id >= 0xbfff {
                20
            } else {
                self.next_id + 1
            };
            if self.bindings.iter().flatten().any(|b| b.id == self.next_id) {
                continue;
            }
            if key.modifiers & MOD_WIN != 0 || key.key == VK_SNAPSHOT as u32 {
                if self.hook.is_none() {
                    self.hook = Some(WinHook::start()?);
                }
                return Ok(Binding {
                    id: self.next_id,
                    key,
                    hooked: true,
                });
            }
            if RegisterHotKey(owner, self.next_id, key.modifiers | MOD_NOREPEAT, key.key) == 0 {
                return Err(format!(
                    "{} is unavailable or used by another app. Choose another shortcut.",
                    key.display()
                ));
            }
            return Ok(Binding {
                id: self.next_id,
                key,
                hooked: false,
            });
        }
        Err("Could not allocate a shortcut ID.".into())
    }
    pub unsafe fn install(&mut self, owner: HWND, keys: &[Hotkey; 3]) -> Vec<String> {
        if let Err(e) = validate(keys) {
            return vec![e];
        }
        let mut errors = Vec::new();
        for (index, key) in keys.iter().copied().enumerate() {
            if key.key == 0 {
                continue;
            }
            match self.acquire(owner, key) {
                Ok(binding) => self.bindings[index] = Some(binding),
                Err(e) => errors.push(e),
            }
        }
        self.sync_hook(owner);
        errors
    }
    /// Reserve all new combinations first. A conflict leaves existing shortcuts live.
    pub unsafe fn replace(&mut self, owner: HWND, keys: &[Hotkey; 3]) -> Result<()> {
        validate(keys)?;
        let mut proposed: [Option<Binding>; 3] = [None; 3];
        let mut acquired: Vec<Binding> = Vec::new();
        for (index, key) in keys.iter().copied().enumerate() {
            if key.key == 0 {
                continue;
            }
            if let Some(binding) = self.bindings.iter().flatten().find(|b| b.key == key) {
                proposed[index] = Some(*binding);
            } else {
                match self.acquire(owner, key) {
                    Ok(binding) => {
                        proposed[index] = Some(binding);
                        acquired.push(binding);
                    }
                    Err(error) => {
                        for binding in acquired {
                            if !binding.hooked {
                                UnregisterHotKey(owner, binding.id);
                            }
                        }
                        self.sync_hook(owner);
                        return Err(error);
                    }
                }
            }
        }
        for binding in self.bindings.iter().flatten() {
            if !proposed.iter().flatten().any(|new| new.id == binding.id) {
                if !binding.hooked {
                    UnregisterHotKey(owner, binding.id);
                }
            }
        }
        self.bindings = proposed;
        self.sync_hook(owner);
        Ok(())
    }
    pub fn action(&self, id: usize) -> Option<usize> {
        self.bindings
            .iter()
            .position(|binding| binding.is_some_and(|b| b.id as usize == id))
    }
    pub unsafe fn clear(&mut self, owner: HWND) {
        self.block_snipping = false;
        for binding in self.bindings.iter().flatten() {
            if !binding.hooked {
                UnregisterHotKey(owner, binding.id);
            }
        }
        self.bindings = [None; 3];
        self.sync_hook(owner);
    }
    pub fn set_block_snipping(&mut self, owner: HWND, enabled: bool) -> Result<()> {
        if enabled && self.hook.is_none() {
            self.hook = Some(WinHook::start()?);
        }
        self.block_snipping = enabled;
        self.sync_hook(owner);
        Ok(())
    }
    pub unsafe fn replace_with_policy(
        &mut self,
        owner: HWND,
        keys: &[Hotkey; 3],
        enabled: bool,
    ) -> Result<()> {
        validate(keys)?;
        let previous = self.block_snipping;
        // Keep an existing hook until the new bindings have been acquired.
        if enabled {
            self.set_block_snipping(owner, true)?;
        }
        if let Err(error) = self.replace(owner, keys) {
            self.set_block_snipping(owner, previous)?;
            return Err(error);
        }
        self.set_block_snipping(owner, enabled)
    }
    fn sync_hook(&mut self, owner: HWND) {
        if self.block_snipping || self.bindings.iter().flatten().any(|b| b.hooked) {
            WinHook::block_snipping(self.block_snipping);
            WinHook::publish(
                owner,
                std::array::from_fn(|i| {
                    self.bindings[i].map(|b| (b.id as u32, b.key)).unwrap_or((
                        0,
                        Hotkey {
                            modifiers: 0,
                            key: 0,
                        },
                    ))
                }),
            );
        } else {
            self.hook = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcut_persistence_and_native_control_roundtrip() {
        for key in DEFAULTS.into_iter().chain([
            Hotkey {
                modifiers: MOD_ALT | MOD_CONTROL,
                key: 0x7b,
            },
            Hotkey {
                modifiers: 0,
                key: 0x2c,
            },
        ]) {
            assert_eq!(Hotkey::parse(&key.encode()), Some(key));
            assert_eq!(Hotkey::from_control(key.control_value()), key);
        }
        assert_eq!(DEFAULTS[0].display(), "Ctrl+Shift+S");
        assert!(Hotkey::parse("bad:5").is_none());
        assert!(Hotkey::parse("16:65").is_none());
        let win = Hotkey {
            modifiers: MOD_WIN | MOD_SHIFT,
            key: 0x53,
        };
        assert_eq!(Hotkey::parse(&win.encode()), Some(win));
        assert_eq!(win.display(), "Win+Shift+S");
        assert_eq!(
            Hotkey {
                modifiers: 0,
                key: VK_SNAPSHOT as u32
            }
            .display(),
            "PrtSc"
        );
        assert!(Hotkey::parse("2:999").is_none());
    }
    #[test]
    fn reject_duplicates_and_plain_typing_keys_allow_disabled_keys() {
        assert!(validate(&[DEFAULTS[0]; 3]).is_err());
        assert!(validate(&[
            Hotkey {
                modifiers: 0,
                key: 65
            },
            DEFAULTS[1],
            DEFAULTS[2]
        ])
        .is_err());
        assert!(validate(
            &[Hotkey {
                modifiers: 0,
                key: 0
            }; 3]
        )
        .is_ok());
    }
    #[test]
    fn replacing_shortcuts_can_swap_modes_and_conflicts_preserve_old_bindings() {
        // Thread-local registrations need no visible window and are removed before return.
        unsafe {
            let owner = std::ptr::null_mut();
            let free: Vec<_> = (0x70..=0x87)
                .filter(|key| *key != VK_F12 as u32)
                .filter_map(|key| {
                    let modifiers = MOD_CONTROL | MOD_ALT | MOD_SHIFT;
                    if RegisterHotKey(owner, 9950, modifiers, key) != 0 {
                        UnregisterHotKey(owner, 9950);
                        Some(Hotkey { modifiers, key })
                    } else {
                        None
                    }
                })
                .take(5)
                .collect();
            assert!(free.len() == 5, "Need five unused test shortcuts");
            let keys = [free[0], free[1], free[2]];
            let mut registry = Hotkeys::new();
            let errors = registry.install(owner, &keys);
            assert!(
                errors.is_empty(),
                "{errors:?}; Win32: {}",
                std::io::Error::last_os_error()
            );
            let swapped = [keys[1], keys[0], keys[2]];
            registry.replace(owner, &swapped).unwrap();
            let before: Vec<_> = registry.bindings.iter().flatten().map(|b| b.id).collect();
            let (ready_tx, ready_rx) = std::sync::mpsc::channel();
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let blocked = free[4];
            let contender = std::thread::spawn(move || {
                let registered =
                    RegisterHotKey(std::ptr::null_mut(), 9000, blocked.modifiers, blocked.key) != 0;
                ready_tx.send(registered).unwrap();
                let _ = done_rx.recv();
                if registered {
                    UnregisterHotKey(std::ptr::null_mut(), 9000);
                }
            });
            assert!(ready_rx.recv().unwrap());
            let mut invalid = swapped;
            invalid[0] = free[3];
            invalid[1] = blocked;
            assert!(registry.replace(owner, &invalid).is_err());
            assert_eq!(
                registry
                    .bindings
                    .iter()
                    .flatten()
                    .map(|b| b.id)
                    .collect::<Vec<_>>(),
                before
            );
            assert_ne!(
                RegisterHotKey(owner, 9951, free[3].modifiers, free[3].key),
                0,
                "Failed replacement leaked a newly reserved shortcut"
            );
            UnregisterHotKey(owner, 9951);
            for (i, binding) in registry.bindings.iter().enumerate() {
                assert_eq!(registry.action(binding.unwrap().id as usize), Some(i));
            }
            registry.clear(owner);
            done_tx.send(()).unwrap();
            contender.join().unwrap();
        }
    }

    #[test]
    fn native_hotkey_field_records_extended_keys_and_can_be_cleared() {
        use crate::platform::wide;
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        unsafe {
            InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_HOTKEY_CLASS,
            });
            let parent = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            assert!(!parent.is_null());
            let input = CreateWindowExW(
                0,
                wide("msctls_hotkey32").as_ptr(),
                std::ptr::null(),
                WS_CHILD,
                0,
                0,
                200,
                30,
                parent,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            assert!(!input.is_null());
            windows_sys::Win32::UI::Shell::SetWindowSubclass(
                input,
                Some(crate::hotkey_input_proc),
                1,
                0,
            );
            for key in DEFAULTS.into_iter().chain([Hotkey {
                modifiers: MOD_ALT | MOD_CONTROL,
                key: VK_LEFT as u32,
            }]) {
                SendMessageW(input, HKM_SETHOTKEY, key.control_value(), 0);
                assert_eq!(
                    Hotkey::from_control(SendMessageW(input, HKM_GETHOTKEY, 0, 0) as usize),
                    key
                );
            }
            SendMessageW(input, HKM_SETHOTKEY, 0, 0);
            assert_eq!(SendMessageW(input, HKM_GETHOTKEY, 0, 0), 0);
            SendMessageW(input, WM_KEYUP, VK_SNAPSHOT as usize, 0);
            assert_eq!(
                Hotkey::from_control(SendMessageW(input, HKM_GETHOTKEY, 0, 0) as usize).key,
                VK_SNAPSHOT as u32
            );
            DestroyWindow(parent);
        }
    }
}
