//! Only configured Win / Print Screen shortcuts are intercepted. The hook runs on its own
//! message thread so encoding a large PNG cannot time out the keyboard hook.
use crate::{hotkey::Hotkey, image::Result};
use std::{
    cell::RefCell,
    ptr::{null, null_mut},
    sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
    thread::JoinHandle,
};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

static OWNER: AtomicIsize = AtomicIsize::new(0);
static BLOCK_SNIPPING: AtomicBool = AtomicBool::new(false);
static KEYS: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];
static IDS: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];

#[derive(Default, Debug, PartialEq)]
struct Decision {
    consume: bool,
    action: Option<usize>,
}

struct KeyboardState {
    block_snipping: bool,
    modifiers: [bool; 8],
    swallowed: [bool; 256],
}
const MODIFIER_KEYS: [u32; 8] = [
    VK_LCONTROL as u32,
    VK_RCONTROL as u32,
    VK_LSHIFT as u32,
    VK_RSHIFT as u32,
    VK_LMENU as u32,
    VK_RMENU as u32,
    VK_LWIN as u32,
    VK_RWIN as u32,
];
impl KeyboardState {
    fn new() -> Self {
        Self {
            block_snipping: false,
            modifiers: [false; 8],
            swallowed: [false; 256],
        }
    }
    fn event(&mut self, key: u32, down: bool, targets: &[u32; 3]) -> Decision {
        if let Some(index) = MODIFIER_KEYS.iter().position(|k| *k == key) {
            self.modifiers[index] = down;
            return Decision::default();
        }
        if key >= 256 {
            return Decision::default();
        }
        if self.swallowed[key as usize] {
            if !down {
                self.swallowed[key as usize] = false;
            }
            return Decision {
                consume: true,
                action: None,
            };
        }
        // Some Print Screen devices report only a release. A normal down/up
        // sequence is deduplicated by swallowed[] above.
        if !down && key != VK_SNAPSHOT as u32 {
            return Decision::default();
        }
        let mut mods = 0;
        for (i, flag) in [MOD_CONTROL, MOD_SHIFT, MOD_ALT, MOD_WIN]
            .into_iter()
            .enumerate()
        {
            if self.modifiers[i * 2] || self.modifiers[i * 2 + 1] {
                mods |= flag;
            }
        }
        if mods & MOD_WIN == 0 && key != VK_SNAPSHOT as u32 {
            return Decision::default();
        }
        if let Some(index) = targets
            .iter()
            .position(|target| *target == ((mods << 8) | key))
        {
            self.swallowed[key as usize] = down;
            return Decision {
                consume: true,
                action: Some(index),
            };
        }
        if self.block_snipping
            && ((key == VK_SNAPSHOT as u32 && mods == 0)
                || (key == 0x53 && mods == (MOD_WIN | MOD_SHIFT)))
        {
            self.swallowed[key as usize] = down;
            return Decision {
                consume: true,
                action: None,
            };
        }
        Decision::default()
    }
}
thread_local! { static STATE: RefCell<KeyboardState> = RefCell::new(KeyboardState::new()); }

unsafe extern "system" fn keyboard_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code != HC_ACTION as i32 {
        return CallNextHookEx(null_mut(), code, w, l);
    }
    let event = &*(l as *const KBDLLHOOKSTRUCT);
    // Keyboard utilities may synthesize Print Screen. Accept that key only;
    // synthetic modifiers and our own menu-mask events remain ignored.
    if event.flags & LLKHF_INJECTED != 0 && event.vkCode != VK_SNAPSHOT as u32 {
        return CallNextHookEx(null_mut(), code, w, l);
    }
    let down = w as u32 == WM_KEYDOWN || w as u32 == WM_SYSKEYDOWN;
    let targets = std::array::from_fn(|i| KEYS[i].load(Ordering::Acquire));
    let (decision, win_held) = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.block_snipping = BLOCK_SNIPPING.load(Ordering::Acquire);
        let decision = state.event(event.vkCode, down, &targets);
        (decision, state.modifiers[6] || state.modifiers[7])
    });
    // Mask Start even when a blocked chord has no FeatherShot action assigned.
    if decision.consume && down && win_held {
        let mut inputs: [INPUT; 2] = std::mem::zeroed();
        for (i, input) in inputs.iter_mut().enumerate() {
            input.r#type = INPUT_KEYBOARD;
            input.Anonymous.ki = KEYBDINPUT {
                wVk: 0xE8,
                dwFlags: if i == 1 { KEYEVENTF_KEYUP } else { 0 },
                ..std::mem::zeroed()
            };
        }
        SendInput(2, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
    if let Some(index) = decision.action {
        let owner = OWNER.load(Ordering::Acquire) as HWND;
        if !owner.is_null() {
            PostMessageW(
                owner,
                WM_HOTKEY,
                IDS[index].load(Ordering::Acquire) as usize,
                0,
            );
        }
    }
    if decision.consume {
        1
    } else {
        CallNextHookEx(null_mut(), code, w, l)
    }
}

pub struct WinHook {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}
impl WinHook {
    pub fn block_snipping(enabled: bool) {
        BLOCK_SNIPPING.store(enabled, Ordering::Release);
    }
    pub fn start() -> Result<Self> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("Win shortcuts".into())
            .spawn(move || unsafe {
                let mut message: MSG = std::mem::zeroed();
                PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
                // Seed held modifier keys before installing; async state is not queried in the callback.
                STATE.with(|state| {
                    let mut state = state.borrow_mut();
                    for (i, key) in MODIFIER_KEYS.iter().enumerate() {
                        state.modifiers[i] = GetAsyncKeyState(*key as i32) < 0;
                    }
                });
                let hook = SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(keyboard_proc),
                    GetModuleHandleW(null()),
                    0,
                );
                if hook.is_null() {
                    let _ = tx.send(Err(format!(
                        "Cannot enable Win shortcuts: {}",
                        std::io::Error::last_os_error()
                    )));
                    return;
                }
                if tx.send(Ok(GetCurrentThreadId())).is_err() {
                    UnhookWindowsHookEx(hook);
                    return;
                }
                while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                UnhookWindowsHookEx(hook);
            })
            .map_err(|e| e.to_string())?;
        match rx.recv().map_err(|e| e.to_string())? {
            Ok(thread_id) => Ok(Self {
                thread_id,
                thread: Some(thread),
            }),
            Err(error) => {
                let _ = thread.join();
                Err(error)
            }
        }
    }
    pub fn publish(owner: HWND, bindings: [(u32, Hotkey); 3]) {
        for key in &KEYS {
            key.store(0, Ordering::Release);
        }
        OWNER.store(owner as isize, Ordering::Release);
        for (i, (id, key)) in bindings.into_iter().enumerate() {
            IDS[i].store(id, Ordering::Release);
            if key.key != 0 && (key.modifiers & MOD_WIN != 0 || key.key == VK_SNAPSHOT as u32) {
                KEYS[i].store((key.modifiers << 8) | key.key, Ordering::Release);
            }
        }
    }
}
impl Drop for WinHook {
    fn drop(&mut self) {
        Self::block_snipping(false);
        for key in &KEYS {
            key.store(0, Ordering::Release);
        }
        OWNER.store(0, Ordering::Release);
        unsafe {
            PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snipping_blocking_preserves_bindings_and_releases_keys() {
        let mut state = KeyboardState::new();
        let targets = [0, 0, VK_SNAPSHOT as u32];
        state.block_snipping = true;
        // Assigned Print Screen still captures; bare Print Screen without a binding only blocks.
        assert_eq!(
            state.event(VK_SNAPSHOT as u32, true, &targets).action,
            Some(2)
        );
        state.event(VK_SNAPSHOT as u32, false, &targets);
        assert!(state.event(VK_SNAPSHOT as u32, false, &[0; 3]).consume);
        state.event(VK_LWIN as u32, true, &targets);
        state.event(VK_LSHIFT as u32, true, &targets);
        assert_eq!(
            state.event(0x53, true, &targets),
            Decision {
                consume: true,
                action: None
            }
        );
        assert_eq!(state.event(0x41, true, &targets), Decision::default());
        // Turning off mid-press still swallows the corresponding key-up.
        state.block_snipping = false;
        assert!(state.event(0x53, false, &targets).consume);
        assert_eq!(state.event(0x53, true, &targets), Decision::default());
        state.block_snipping = true;
        let assigned = [((MOD_WIN | MOD_SHIFT) << 8) | 0x53, 0, 0];
        assert_eq!(state.event(0x53, true, &assigned).action, Some(0));
        state.event(0x53, false, &assigned);
        state.event(VK_LCONTROL as u32, true, &assigned);
        assert_eq!(state.event(0x53, true, &assigned), Decision::default());
    }
    #[test]
    fn native_print_screen_dispatches_once_to_hidden_test_window() {
        unsafe {
            struct TestWindow(HWND);
            impl Drop for TestWindow {
                fn drop(&mut self) {
                    unsafe {
                        DestroyWindow(self.0);
                    }
                }
            }
            let owner = TestWindow(CreateWindowExW(
                0,
                crate::platform::wide("STATIC").as_ptr(),
                null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                null_mut(),
                null(),
            ));
            assert!(!owner.0.is_null());
            let mut registry = crate::hotkey::Hotkeys::new();
            let disabled = Hotkey {
                modifiers: 0,
                key: 0,
            };
            registry
                .replace(
                    owner.0,
                    &[
                        Hotkey {
                            modifiers: 0,
                            key: VK_SNAPSHOT as u32,
                        },
                        disabled,
                        disabled,
                    ],
                )
                .unwrap();
            let mut inputs: [INPUT; 2] = std::mem::zeroed();
            for (index, input) in inputs.iter_mut().enumerate() {
                input.r#type = INPUT_KEYBOARD;
                input.Anonymous.ki = KEYBDINPUT {
                    wVk: VK_SNAPSHOT,
                    dwFlags: KEYEVENTF_EXTENDEDKEY | if index == 1 { KEYEVENTF_KEYUP } else { 0 },
                    ..std::mem::zeroed()
                };
            }
            assert_eq!(
                SendInput(2, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32),
                2
            );
            let start = std::time::Instant::now();
            let mut received = 0;
            let mut message: MSG = std::mem::zeroed();
            while start.elapsed() < std::time::Duration::from_millis(500) {
                while PeekMessageW(&mut message, owner.0, WM_HOTKEY, WM_HOTKEY, PM_REMOVE) != 0 {
                    assert_eq!(registry.action(message.wParam), Some(0));
                    received += 1;
                }
                if received > 0 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
            while PeekMessageW(&mut message, owner.0, WM_HOTKEY, WM_HOTKEY, PM_REMOVE) != 0 {
                received += 1;
            }
            registry.clear(owner.0);
            assert_eq!(
                received, 1,
                "Print Screen must dispatch exactly one capture action"
            );
        }
    }
    #[test]
    fn print_screen_accepts_release_only_and_deduplicates_normal_press() {
        let mut state = KeyboardState::new();
        let key = VK_SNAPSHOT as u32;
        let targets = [key, 0, 0];
        assert_eq!(state.event(key, true, &targets).action, Some(0));
        assert_eq!(
            state.event(key, true, &targets),
            Decision {
                consume: true,
                action: None
            }
        );
        assert_eq!(
            state.event(key, false, &targets),
            Decision {
                consume: true,
                action: None
            }
        );
        assert_eq!(
            state.event(key, false, &targets),
            Decision {
                consume: true,
                action: Some(0)
            }
        );
        assert_eq!(state.event(0x41, true, &targets), Decision::default());
        state.event(VK_LSHIFT as u32, true, &targets);
        assert_eq!(state.event(key, true, &targets), Decision::default());
        assert_eq!(state.event(key, false, &targets), Decision::default());
    }
    #[test]
    fn hook_thread_installs_and_unhooks_without_an_active_shortcut() {
        let hook = WinHook::start().expect("Install Windows keyboard hook");
        assert_ne!(hook.thread_id, 0);
        drop(hook);
        assert_eq!(OWNER.load(Ordering::Acquire), 0);
        assert!(KEYS.iter().all(|key| key.load(Ordering::Acquire) == 0));
    }
    #[test]
    fn win_shift_s_consumes_only_matching_chord_and_one_action_per_press() {
        let mut state = KeyboardState::new();
        let targets = [((MOD_WIN | MOD_SHIFT) << 8) | 0x53, 0, 0];
        assert_eq!(
            state.event(VK_LWIN as u32, true, &targets),
            Decision::default()
        );
        assert_eq!(state.event(0x41, true, &targets), Decision::default()); // Win+A still belongs to Windows.
        state.event(VK_RSHIFT as u32, true, &targets);
        assert_eq!(
            state.event(0x53, true, &targets),
            Decision {
                consume: true,
                action: Some(0)
            }
        );
        assert_eq!(
            state.event(0x53, true, &targets),
            Decision {
                consume: true,
                action: None
            }
        );
        assert_eq!(
            state.event(VK_LWIN as u32, false, &targets),
            Decision::default()
        );
        assert_eq!(
            state.event(VK_RSHIFT as u32, false, &targets),
            Decision::default()
        );
        assert_eq!(
            state.event(0x53, false, &targets),
            Decision {
                consume: true,
                action: None
            }
        );
        assert_eq!(state.event(0x53, true, &targets), Decision::default()); // Normal typing restored.
    }
    #[test]
    fn right_win_and_extra_modifiers_match_exactly() {
        let mut state = KeyboardState::new();
        let targets = [0, ((MOD_WIN | MOD_ALT) << 8) | 0x46, 0];
        state.event(VK_RWIN as u32, true, &targets);
        state.event(VK_LMENU as u32, true, &targets);
        state.event(VK_LCONTROL as u32, true, &targets);
        assert_eq!(state.event(0x46, true, &targets), Decision::default());
        state.event(VK_LCONTROL as u32, false, &targets);
        assert_eq!(state.event(0x46, true, &targets).action, Some(1));
    }
}
