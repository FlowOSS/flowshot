//! Windows global hotkeys implementation via Win32 `RegisterHotKey`.

use crate::command::CommandSink;
use crate::state::DaemonState;
use std::sync::Arc;

/// Windows global hotkeys listener.
#[derive(Debug)]
pub struct WindowsShortcuts {
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl WindowsShortcuts {
    /// Spawns the Windows global hotkey listener thread.
    #[must_use]
    pub fn spawn(sink: Arc<dyn CommandSink>, state: Arc<DaemonState>) -> Self {
        #[cfg(windows)]
        {
            spawn_windows_hotkeys(sink, state)
        }
        #[cfg(not(windows))]
        {
            let _ = (sink, state);
            Self { shutdown: None }
        }
    }

    /// Shuts down the hotkey listener and unregisters keys.
    pub fn shutdown(self) {
        if let Some(tx) = self.shutdown {
            let _ = tx.send(());
        }
    }
}

#[cfg(windows)]
#[expect(
    clippy::too_many_lines,
    clippy::borrow_as_ptr,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::default_trait_access,
    reason = "Win32 hotkey FFI"
)]
fn spawn_windows_hotkeys(sink: Arc<dyn CommandSink>, state: Arc<DaemonState>) -> WindowsShortcuts {
    let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();

    std::thread::Builder::new()
        .name("flowshot-win-hotkeys".to_owned())
        .spawn(move || {
            use std::mem;
            use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
            use windows::Win32::UI::Input::KeyboardAndMouse::{
                MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey, VK_SNAPSHOT,
            };
            use windows::Win32::UI::WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, MSG, PM_REMOVE,
                PeekMessageW, RegisterClassW, TranslateMessage, WM_HOTKEY, WNDCLASSW,
                WS_OVERLAPPED,
            };
            use windows::core::w;

            unsafe extern "system" fn wnd_proc(
                hwnd: HWND,
                msg: u32,
                wparam: WPARAM,
                lparam: LPARAM,
            ) -> LRESULT {
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }

            const ID_PRTSC_REGION: i32 = 1001;
            const ID_PRTSC_FULL: i32 = 1002;

            let class_name = w!("FlowShotHotkeyClass");
            let wnd_class = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                lpszClassName: class_name,
                ..unsafe { mem::zeroed() }
            };
            unsafe { RegisterClassW(&wnd_class) };

            let hwnd = unsafe {
                CreateWindowExW(
                    Default::default(),
                    class_name,
                    w!("FlowShotHotkeyWindow"),
                    WS_OVERLAPPED,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    None,
                    None,
                )
            };
            let Ok(hwnd) = hwnd else {
                return;
            };

            let reg1 = unsafe {
                RegisterHotKey(
                    Some(hwnd),
                    ID_PRTSC_REGION,
                    MOD_NOREPEAT,
                    VK_SNAPSHOT.0 as u32,
                )
            };

            let reg2 = unsafe {
                RegisterHotKey(
                    Some(hwnd),
                    ID_PRTSC_FULL,
                    MOD_NOREPEAT | MOD_SHIFT,
                    VK_SNAPSHOT.0 as u32,
                )
            };

            if reg1.is_ok() || reg2.is_ok() {
                state.set_shortcuts_registered(true);
            }

            let mut msg: MSG = unsafe { mem::zeroed() };
            loop {
                if rx.try_recv().is_ok() {
                    break;
                }

                let has_msg =
                    unsafe { PeekMessageW(&mut msg, Some(hwnd), 0, 0, PM_REMOVE).as_bool() };

                if has_msg {
                    if msg.message == WM_HOTKEY {
                        match msg.wParam.0 as i32 {
                            ID_PRTSC_REGION => {
                                sink.dispatch(crate::command::DaemonCommand::Capture(
                                    crate::request::CaptureRequest::default(),
                                ));
                            }
                            ID_PRTSC_FULL => {
                                sink.dispatch(crate::command::DaemonCommand::CaptureFull);
                            }
                            _ => {}
                        }
                    }
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }

            unsafe {
                let _ = UnregisterHotKey(Some(hwnd), ID_PRTSC_REGION);
                let _ = UnregisterHotKey(Some(hwnd), ID_PRTSC_FULL);
                let _ = DestroyWindow(hwnd);
            }
            state.set_shortcuts_registered(false);
        })
        .ok();

    WindowsShortcuts { shutdown: Some(tx) }
}
