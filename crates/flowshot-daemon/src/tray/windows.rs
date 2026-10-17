//! Windows system tray implementation via Win32 `Shell_NotifyIconW`.

use crate::command::CommandSink;
use crate::notify::Notifier;
use crate::state::DaemonState;
use std::sync::Arc;

/// Windows system tray host.
#[derive(Debug)]
pub struct WindowsTray {
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl WindowsTray {
    /// Spawns the Windows system tray host.
    #[must_use]
    pub fn spawn(
        sink: Arc<dyn CommandSink>,
        state: Arc<DaemonState>,
        notifier: Arc<dyn Notifier>,
    ) -> Self {
        #[cfg(windows)]
        {
            spawn_windows_tray(sink, state, notifier)
        }
        #[cfg(not(windows))]
        {
            let _ = (sink, state, notifier);
            Self { shutdown: None }
        }
    }

    /// Shuts down the tray and removes the icon.
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
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::default_trait_access,
    reason = "Win32 tray FFI"
)]
fn spawn_windows_tray(
    sink: Arc<dyn CommandSink>,
    state: Arc<DaemonState>,
    _notifier: Arc<dyn Notifier>,
) -> WindowsTray {
    let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();

    std::thread::Builder::new()
        .name("flowshot-win-tray".to_owned())
        .spawn(move || {
            use std::mem;
            use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
            use windows::Win32::UI::Shell::{
                NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
            };
            use windows::Win32::UI::WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, DefWindowProcW, DestroyMenu, DestroyWindow,
                DispatchMessageW, GetCursorPos, MF_SEPARATOR, MF_STRING, MSG, PM_REMOVE,
                PeekMessageW, RegisterClassW, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD,
                TrackPopupMenuEx, TranslateMessage, WM_APP, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
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

            const WM_TRAY: u32 = WM_APP + 1;
            const CMD_CAPTURE: usize = 1;
            const CMD_SETTINGS: usize = 2;
            const CMD_QUIT: usize = 3;

            let class_name = w!("FlowShotTrayClass");
            let wnd_class = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                lpszClassName: class_name,
                ..unsafe { mem::zeroed() }
            };
            unsafe { RegisterClassW(&wnd_class) };

            let hwnd = unsafe {
                windows::Win32::UI::WindowsAndMessaging::CreateWindowExW(
                    Default::default(),
                    class_name,
                    w!("FlowShotTrayWindow"),
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

            let mut nid: NOTIFYICONDATAW = unsafe { mem::zeroed() };
            nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_MESSAGE | NIF_TIP;
            nid.uCallbackMessage = WM_TRAY;

            let tip = "FlowShot\0".encode_utf16().collect::<Vec<u16>>();
            for (dst, src) in nid.szTip.iter_mut().zip(&tip) {
                *dst = *src;
            }

            unsafe {
                let _ = Shell_NotifyIconW(NIM_ADD, &nid);
            };
            state.set_tray(true);

            let mut msg: MSG = unsafe { mem::zeroed() };
            loop {
                if rx.try_recv().is_ok() {
                    break;
                }

                let has_msg =
                    unsafe { PeekMessageW(&mut msg, Some(hwnd), 0, 0, PM_REMOVE).as_bool() };

                if has_msg {
                    if msg.message == WM_TRAY {
                        match msg.lParam.0 as u32 {
                            WM_LBUTTONUP => {
                                sink.dispatch(crate::command::DaemonCommand::Capture(
                                    crate::request::CaptureRequest::default(),
                                ));
                            }
                            WM_RBUTTONUP => {
                                let mut pt = POINT::default();
                                unsafe {
                                    let _ = GetCursorPos(&mut pt);
                                };
                                let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
                                    continue;
                                };
                                unsafe {
                                    let _ = AppendMenuW(
                                        menu,
                                        MF_STRING,
                                        CMD_CAPTURE,
                                        w!("Take Screenshot"),
                                    );
                                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
                                    let _ =
                                        AppendMenuW(menu, MF_STRING, CMD_SETTINGS, w!("Settings"));
                                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
                                    let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, w!("Quit"));
                                }
                                let cmd = unsafe {
                                    TrackPopupMenuEx(
                                        menu,
                                        TPM_RETURNCMD.0 | TPM_LEFTALIGN.0 | TPM_BOTTOMALIGN.0,
                                        pt.x,
                                        pt.y,
                                        hwnd,
                                        None,
                                    )
                                };
                                unsafe {
                                    let _ = DestroyMenu(menu);
                                };

                                match cmd.0 as usize {
                                    CMD_CAPTURE => {
                                        sink.dispatch(crate::command::DaemonCommand::Capture(
                                            crate::request::CaptureRequest::default(),
                                        ));
                                    }
                                    CMD_SETTINGS => {
                                        sink.dispatch(crate::command::DaemonCommand::Settings);
                                    }
                                    CMD_QUIT => {
                                        break;
                                    }
                                    _ => {}
                                }
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

            state.set_tray(false);
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                let _ = DestroyWindow(hwnd);
            }
        })
        .ok();

    WindowsTray { shutdown: Some(tx) }
}
