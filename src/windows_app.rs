use std::ptr::{null, null_mut};
use std::time::{SystemTime, UNIX_EPOCH};

use zeroize::Zeroize;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, GlobalFree, ERROR_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM,
    LRESULT, POINT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, GetStockObject, UpdateWindow, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_GUI_FONT, FF_DONTCARE, FW_BOLD,
    OUT_DEFAULT_PRECIS,
};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NOTIFYICONDATAW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
    NIM_MODIFY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::{autostart, secret_store, totp};

const APP_NAME: &str = "SIT TOTP";
const CLASS_NAME: &str = "SIT_TOTP_FOR_WINDOWS_NATIVE";
const MUTEX_NAME: &str = "Local\\SIT_TOTP_FOR_WINDOWS_INSTANCE";

// Stable Win32 values kept local so extra API feature groups are unnecessary.
const COLOR_WINDOW_BRUSH: isize = 6; // COLOR_WINDOW + 1
const STATIC_CENTER_STYLE: u32 = 0x0000_0001; // SS_CENTER
const EDIT_SET_LIMIT_TEXT: u32 = 0x00C5; // EDIT_SET_LIMIT_TEXT
const BUTTON_UNCHECKED: usize = 0; // BST_UNCHECKED
const BUTTON_CHECKED: usize = 1; // BST_CHECKED
const CLIPBOARD_UNICODE_TEXT: u32 = 13; // CLIPBOARD_UNICODE_TEXT

const WM_TRAY: u32 = WM_APP + 1;
const WM_SHOW_EXISTING: u32 = WM_APP + 2;
const TIMER_ID: usize = 1;
const HOTKEY_ID: i32 = 1;
const TRAY_ID: u32 = 1;

const ID_CODE: i32 = 1001;
const ID_COUNTDOWN: i32 = 1002;
const ID_COPY: i32 = 1003;
const ID_SEED: i32 = 1004;
const ID_SAVE: i32 = 1005;
const ID_DELETE: i32 = 1006;
const ID_AUTOSTART: i32 = 1007;
const ID_STATUS: i32 = 1008;

const MENU_OPEN: usize = 2001;
const MENU_COPY: usize = 2002;
const MENU_AUTOSTART: usize = 2003;
const MENU_DELETE: usize = 2004;
const MENU_EXIT: usize = 2005;

struct AppState {
    hwnd: HWND,
    code_label: HWND,
    countdown_label: HWND,
    seed_edit: HWND,
    status_label: HWND,
    autostart_checkbox: HWND,
    code_font: isize,
    tray_icon: NOTIFYICONDATAW,
    seed: Option<String>,
    last_code: String,
    exiting: bool,
}

impl Drop for AppState {
    fn drop(&mut self) {
        if let Some(seed) = self.seed.as_mut() {
            seed.zeroize();
        }
        if self.code_font != 0 {
            unsafe {
                DeleteObject(self.code_font as _);
            }
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_into<const N: usize>(destination: &mut [u16; N], value: &str) {
    destination.fill(0);
    for (target, source) in destination
        .iter_mut()
        .zip(value.encode_utf16().take(N.saturating_sub(1)))
    {
        *target = source;
    }
}

unsafe fn state_from(hwnd: HWND) -> Option<&'static mut AppState> {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppState;
    pointer.as_mut()
}

unsafe fn create_control(
    class_name: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    hwnd: HWND,
    id: i32,
    instance: HINSTANCE,
) -> HWND {
    let class_name = wide(class_name);
    let text = wide(text);
    CreateWindowExW(
        0,
        class_name.as_ptr(),
        text.as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        x,
        y,
        width,
        height,
        hwnd,
        id as isize as _,
        instance,
        null(),
    )
}

unsafe fn set_text(hwnd: HWND, text: &str) {
    let text = wide(text);
    SetWindowTextW(hwnd, text.as_ptr());
}

unsafe fn get_text(hwnd: HWND) -> String {
    let length = GetWindowTextLengthW(hwnd);
    let mut buffer = vec![0u16; length as usize + 1];
    let copied = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..copied as usize])
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn run() -> Result<(), String> {
    unsafe {
        let mutex_name = wide(MUTEX_NAME);
        let mutex = CreateMutexW(null(), 0, mutex_name.as_ptr());
        if mutex.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let class_name = wide(CLASS_NAME);
            let existing = FindWindowW(class_name.as_ptr(), null());
            if !existing.is_null() {
                PostMessageW(existing, WM_SHOW_EXISTING, 0, 0);
            }
            CloseHandle(mutex);
            return Ok(());
        }

        let instance = GetModuleHandleW(null());
        if instance.is_null() {
            CloseHandle(mutex);
            return Err(std::io::Error::last_os_error().to_string());
        }

        let class_name = wide(CLASS_NAME);
        let cursor = LoadCursorW(null_mut(), IDC_ARROW);
        let icon = LoadIconW(null_mut(), IDI_APPLICATION);
        let window_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hIcon: icon,
            hCursor: cursor,
            hbrBackground: COLOR_WINDOW_BRUSH as _,
            lpszClassName: class_name.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&window_class) == 0 {
            CloseHandle(mutex);
            return Err(std::io::Error::last_os_error().to_string());
        }

        let seed = secret_store::load().unwrap_or(None);
        let state = Box::new(AppState {
            hwnd: null_mut(),
            code_label: null_mut(),
            countdown_label: null_mut(),
            seed_edit: null_mut(),
            status_label: null_mut(),
            autostart_checkbox: null_mut(),
            code_font: 0,
            tray_icon: std::mem::zeroed(),
            seed,
            last_code: String::new(),
            exiting: false,
        });
        let state_pointer = Box::into_raw(state);

        let title = wide(APP_NAME);
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            460,
            330,
            null_mut(),
            null_mut(),
            instance,
            state_pointer.cast(),
        );
        if hwnd.is_null() {
            drop(Box::from_raw(state_pointer));
            CloseHandle(mutex);
            return Err(std::io::Error::last_os_error().to_string());
        }

        let background = std::env::args().any(|argument| argument == "--background");
        if !background || (*state_pointer).seed.is_none() {
            ShowWindow(hwnd, SW_SHOW);
            UpdateWindow(hwnd);
        }

        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        CloseHandle(mutex);
        Ok(())
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        let state = create.lpCreateParams as *mut AppState;
        (*state).hwnd = hwnd;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
    }

    match message {
        WM_CREATE => {
            if let Some(state) = state_from(hwnd) {
                initialize_window(state);
                update_code(state);
            }
            0
        }
        WM_COMMAND => {
            if let Some(state) = state_from(hwnd) {
                handle_command(state, (wparam & 0xffff) as i32);
            }
            0
        }
        WM_TIMER => {
            if wparam == TIMER_ID {
                if let Some(state) = state_from(hwnd) {
                    update_code(state);
                }
            }
            0
        }
        WM_HOTKEY => {
            if wparam as i32 == HOTKEY_ID {
                if let Some(state) = state_from(hwnd) {
                    copy_current_code(state);
                }
            }
            0
        }
        WM_TRAY => {
            if let Some(state) = state_from(hwnd) {
                match lparam as u32 {
                    WM_LBUTTONUP | WM_LBUTTONDBLCLK => show_window(state),
                    WM_RBUTTONUP | WM_CONTEXTMENU => show_tray_menu(state),
                    _ => {}
                }
            }
            0
        }
        WM_SHOW_EXISTING => {
            if let Some(state) = state_from(hwnd) {
                show_window(state);
            }
            0
        }
        WM_CLOSE => {
            if let Some(state) = state_from(hwnd) {
                if state.exiting {
                    DestroyWindow(hwnd);
                } else {
                    ShowWindow(hwnd, SW_HIDE);
                }
            }
            0
        }
        WM_DESTROY => {
            if let Some(state) = state_from(hwnd) {
                Shell_NotifyIconW(NIM_DELETE, &state.tray_icon);
            }
            KillTimer(hwnd, TIMER_ID);
            UnregisterHotKey(hwnd, HOTKEY_ID);
            PostQuitMessage(0);
            0
        }
        WM_NCDESTROY => {
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppState;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !pointer.is_null() {
                drop(Box::from_raw(pointer));
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn initialize_window(state: &mut AppState) {
    let instance = GetModuleHandleW(null());
    let default_font = GetStockObject(DEFAULT_GUI_FONT) as isize;

    let title = create_control(
        "STATIC",
        "現在の認証コード",
        STATIC_CENTER_STYLE,
        20,
        18,
        405,
        24,
        state.hwnd,
        0,
        instance,
    );
    state.code_label = create_control(
        "STATIC",
        "------",
        STATIC_CENTER_STYLE,
        20,
        45,
        405,
        55,
        state.hwnd,
        ID_CODE,
        instance,
    );
    state.countdown_label = create_control(
        "STATIC",
        "残り -- 秒",
        STATIC_CENTER_STYLE,
        20,
        102,
        405,
        24,
        state.hwnd,
        ID_COUNTDOWN,
        instance,
    );
    let copy_button = create_control(
        "BUTTON",
        "コードをコピー",
        BS_DEFPUSHBUTTON as u32,
        145,
        130,
        155,
        34,
        state.hwnd,
        ID_COPY,
        instance,
    );

    let seed_label = create_control(
        "STATIC",
        "TOTPシード（Base32 または otpauth URI）",
        0,
        20,
        178,
        405,
        20,
        state.hwnd,
        0,
        instance,
    );
    state.seed_edit = create_control(
        "EDIT",
        "",
        WS_BORDER | ES_AUTOHSCROLL as u32 | ES_PASSWORD as u32,
        20,
        201,
        280,
        25,
        state.hwnd,
        ID_SEED,
        instance,
    );
    SendMessageW(state.seed_edit, EDIT_SET_LIMIT_TEXT, 1024, 0);
    let save_button = create_control(
        "BUTTON",
        "保存",
        BS_PUSHBUTTON as u32,
        310,
        199,
        55,
        29,
        state.hwnd,
        ID_SAVE,
        instance,
    );
    let delete_button = create_control(
        "BUTTON",
        "削除",
        BS_PUSHBUTTON as u32,
        370,
        199,
        55,
        29,
        state.hwnd,
        ID_DELETE,
        instance,
    );
    state.autostart_checkbox = create_control(
        "BUTTON",
        "Windowsログオン時に自動起動する",
        BS_AUTOCHECKBOX as u32,
        20,
        238,
        280,
        24,
        state.hwnd,
        ID_AUTOSTART,
        instance,
    );
    state.status_label = create_control(
        "STATIC",
        "Ctrl + Alt + T で現在のコードをコピー",
        0,
        20,
        268,
        405,
        20,
        state.hwnd,
        ID_STATUS,
        instance,
    );

    for control in [
        title,
        state.countdown_label,
        copy_button,
        seed_label,
        state.seed_edit,
        save_button,
        delete_button,
        state.autostart_checkbox,
        state.status_label,
    ] {
        SendMessageW(control, WM_SETFONT, default_font as usize, 1);
    }

    let face = wide("Segoe UI");
    state.code_font = CreateFontW(
        42,
        0,
        0,
        0,
        FW_BOLD as i32,
        0,
        0,
        0,
        u32::from(DEFAULT_CHARSET),
        u32::from(OUT_DEFAULT_PRECIS),
        u32::from(CLIP_DEFAULT_PRECIS),
        u32::from(CLEARTYPE_QUALITY),
        u32::from(FF_DONTCARE),
        face.as_ptr(),
    ) as isize;
    if state.code_font != 0 {
        SendMessageW(state.code_label, WM_SETFONT, state.code_font as usize, 1);
    }

    SendMessageW(
        state.autostart_checkbox,
        BM_SETCHECK,
        if autostart::is_enabled() {
            BUTTON_CHECKED
        } else {
            BUTTON_UNCHECKED
        },
        0,
    );

    let icon = LoadIconW(null_mut(), IDI_APPLICATION);
    state.tray_icon.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    state.tray_icon.hWnd = state.hwnd;
    state.tray_icon.uID = TRAY_ID;
    state.tray_icon.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    state.tray_icon.uCallbackMessage = WM_TRAY;
    state.tray_icon.hIcon = icon;
    copy_into(&mut state.tray_icon.szTip, "SIT TOTP");
    Shell_NotifyIconW(NIM_ADD, &state.tray_icon);

    RegisterHotKey(
        state.hwnd,
        HOTKEY_ID,
        MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
        b'T' as u32,
    );
    SetTimer(state.hwnd, TIMER_ID, 250, None);

    if state.seed.is_some() {
        set_text(state.status_label, "シードはWindows DPAPIで暗号化して保存されています");
    } else {
        set_text(state.status_label, "シードを入力して保存してください");
    }
}

unsafe fn handle_command(state: &mut AppState, command: i32) {
    match command {
        ID_COPY => copy_current_code(state),
        ID_SAVE => save_seed(state),
        ID_DELETE => delete_seed(state),
        ID_AUTOSTART => {
            let enabled = SendMessageW(state.autostart_checkbox, BM_GETCHECK, 0, 0)
                == BUTTON_CHECKED as isize;
            match autostart::set_enabled(enabled) {
                Ok(()) => set_text(
                    state.status_label,
                    if enabled {
                        "自動起動を有効にしました"
                    } else {
                        "自動起動を無効にしました"
                    },
                ),
                Err(error) => {
                    SendMessageW(
                        state.autostart_checkbox,
                        BM_SETCHECK,
                        if enabled {
                            BUTTON_UNCHECKED
                        } else {
                            BUTTON_CHECKED
                        },
                        0,
                    );
                    show_error(state.hwnd, &error);
                }
            }
        }
        command if command as usize == MENU_OPEN => show_window(state),
        command if command as usize == MENU_COPY => copy_current_code(state),
        command if command as usize == MENU_AUTOSTART => {
            let enabled = !autostart::is_enabled();
            if let Err(error) = autostart::set_enabled(enabled) {
                show_error(state.hwnd, &error);
            } else {
                SendMessageW(
                    state.autostart_checkbox,
                    BM_SETCHECK,
                    if enabled {
                        BUTTON_CHECKED
                    } else {
                        BUTTON_UNCHECKED
                    },
                    0,
                );
            }
        }
        command if command as usize == MENU_DELETE => delete_seed(state),
        command if command as usize == MENU_EXIT => {
            state.exiting = true;
            DestroyWindow(state.hwnd);
        }
        _ => {}
    }
}

unsafe fn save_seed(state: &mut AppState) {
    let raw = get_text(state.seed_edit);
    match totp::normalize_seed(&raw) {
        Ok(mut normalized) => {
            if let Err(error) = secret_store::save(&normalized) {
                normalized.zeroize();
                show_error(state.hwnd, &error);
                return;
            }
            if let Some(seed) = state.seed.as_mut() {
                seed.zeroize();
            }
            state.seed = Some(normalized);
            SetWindowTextW(state.seed_edit, wide("").as_ptr());
            set_text(state.status_label, "シードをDPAPIで暗号化して保存しました");
            update_code(state);
        }
        Err(error) => show_error(state.hwnd, &error),
    }
}

unsafe fn delete_seed(state: &mut AppState) {
    let prompt = wide("保存済みのTOTPシードを削除しますか？");
    let title = wide(APP_NAME);
    if MessageBoxW(
        state.hwnd,
        prompt.as_ptr(),
        title.as_ptr(),
        MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
    ) != IDYES
    {
        return;
    }

    match secret_store::delete() {
        Ok(()) => {
            if let Some(seed) = state.seed.as_mut() {
                seed.zeroize();
            }
            state.seed = None;
            state.last_code.clear();
            set_text(state.code_label, "------");
            set_text(state.countdown_label, "シード未設定");
            set_text(state.status_label, "シードを削除しました");
        }
        Err(error) => show_error(state.hwnd, &error),
    }
}

unsafe fn update_code(state: &mut AppState) {
    let now = now_seconds();
    let remaining = totp::remaining_seconds(now);
    set_text(state.countdown_label, &format!("残り {remaining} 秒"));

    if let Some(seed) = state.seed.as_deref() {
        match totp::generate(seed, now) {
            Ok(code) => {
                if state.last_code != code {
                    state.last_code = code;
                    set_text(state.code_label, &state.last_code);
                }
                copy_into(
                    &mut state.tray_icon.szTip,
                    &format!("SIT TOTP — 残り {remaining} 秒"),
                );
                Shell_NotifyIconW(NIM_MODIFY, &state.tray_icon);
            }
            Err(error) => set_text(state.status_label, &error),
        }
    } else {
        set_text(state.code_label, "------");
        set_text(state.countdown_label, "シード未設定");
    }
}

unsafe fn copy_current_code(state: &mut AppState) {
    if state.seed.is_none() || state.last_code.len() != 6 {
        set_text(state.status_label, "先にTOTPシードを設定してください");
        show_window(state);
        return;
    }

    match set_clipboard_text(state.hwnd, &state.last_code) {
        Ok(()) => set_text(state.status_label, "認証コードをクリップボードへコピーしました"),
        Err(error) => show_error(state.hwnd, &error),
    }
}

unsafe fn set_clipboard_text(hwnd: HWND, text: &str) -> Result<(), String> {
    if OpenClipboard(hwnd) == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if EmptyClipboard() == 0 {
        CloseClipboard();
        return Err(std::io::Error::last_os_error().to_string());
    }

    let data = wide(text);
    let byte_len = data.len() * std::mem::size_of::<u16>();
    let memory = GlobalAlloc(GMEM_MOVEABLE, byte_len);
    if memory.is_null() {
        CloseClipboard();
        return Err("クリップボード用メモリを確保できません".to_owned());
    }

    let pointer = GlobalLock(memory) as *mut u16;
    if pointer.is_null() {
        GlobalFree(memory);
        CloseClipboard();
        return Err(std::io::Error::last_os_error().to_string());
    }
    std::ptr::copy_nonoverlapping(data.as_ptr(), pointer, data.len());
    GlobalUnlock(memory);

    if SetClipboardData(CLIPBOARD_UNICODE_TEXT, memory).is_null() {
        GlobalFree(memory);
        CloseClipboard();
        return Err(std::io::Error::last_os_error().to_string());
    }
    CloseClipboard();
    Ok(())
}

unsafe fn show_window(state: &mut AppState) {
    ShowWindow(state.hwnd, SW_RESTORE);
    SetForegroundWindow(state.hwnd);
}

unsafe fn show_tray_menu(state: &mut AppState) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }

    let open = wide("開く");
    let copy = wide("コードをコピー\tCtrl+Alt+T");
    let autostart = wide(if autostart::is_enabled() {
        "自動起動を無効にする"
    } else {
        "自動起動を有効にする"
    });
    let delete = wide("保存済みシードを削除");
    let exit = wide("終了");

    AppendMenuW(menu, MF_STRING, MENU_OPEN, open.as_ptr());
    AppendMenuW(menu, MF_STRING, MENU_COPY, copy.as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    AppendMenuW(menu, MF_STRING, MENU_AUTOSTART, autostart.as_ptr());
    AppendMenuW(menu, MF_STRING, MENU_DELETE, delete.as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    AppendMenuW(menu, MF_STRING, MENU_EXIT, exit.as_ptr());

    let mut point = POINT { x: 0, y: 0 };
    GetCursorPos(&mut point);
    SetForegroundWindow(state.hwnd);
    let command = TrackPopupMenu(
        menu,
        TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
        point.x,
        point.y,
        0,
        state.hwnd,
        null(),
    );
    DestroyMenu(menu);

    if command != 0 {
        handle_command(state, command as i32);
    }
}

unsafe fn show_error(hwnd: HWND, message: &str) {
    let title = wide(APP_NAME);
    let message = wide(message);
    MessageBoxW(hwnd, message.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
}

pub fn show_fatal_error(message: &str) {
    unsafe {
        show_error(null_mut(), message);
    }
}
