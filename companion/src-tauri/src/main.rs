//! Windows 桌面入口：只出 Windows 版（HID 走 Win32 API，无跨平台目标）。

fn main() {
    companion_lib::run()
}
