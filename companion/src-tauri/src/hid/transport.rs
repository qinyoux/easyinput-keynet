//! HID 输入报告读取（自 easyop `transport.rs::read_input_report_once` 原样移植）。
//!
//! Companion **只读** 0x11 输入报告（Host Action 上报），不下发任何 Feature
//! Report —— 按键配置由 easyop 负责（方案 D：固件零改动）。因此这里只移植
//! OVERLAPPED 读路径，去掉 0x10 分块下发与 Ack 等待逻辑。
//!
//! ⚠️ 超时后必须 `CancelIoEx` 并等待取消完成：否则句柄上残留一个悬挂的后台读，
//! 下一次迭代再发起 ReadFile 就是"同一句柄两个并发读"，HID 驱动会把每条报告
//! 投递给最老的那个读，导致所有报告被吞掉（现象：按了键没反应，像设备掉线）。

use std::time::Duration;

use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::ReadFile;
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use crate::constants::HRESULT_ERROR_IO_PENDING;

/// 阻塞读取一条 HID 输入报告（含 Report ID 首字节）。超时返回 `Ok(None)`。
pub fn read_input_report_once(handle: HANDLE, timeout_ms: u32) -> Result<Option<Vec<u8>>, String> {
    let event = unsafe { CreateEventW(None, true, false, windows::core::PCWSTR::null()) }
        .map_err(|e| format!("CreateEventW: {e}"))?;
    let mut overlapped = OVERLAPPED::default();
    overlapped.hEvent = event;
    let mut buf = [0u8; 64];
    let mut bytes = 0u32;
    let res = unsafe {
        ReadFile(
            handle,
            Some(&mut buf),
            Some(&mut bytes as *mut u32),
            Some(&mut overlapped as *mut OVERLAPPED),
        )
    };
    let transferred = match res {
        Ok(()) => bytes,
        Err(e) => {
            if e.code().0 as u32 == HRESULT_ERROR_IO_PENDING {
                let wait = unsafe { WaitForSingleObject(event, timeout_ms) };
                if wait == WAIT_OBJECT_0 {
                    let mut n = 0u32;
                    unsafe { GetOverlappedResult(handle, &overlapped, &mut n, false) }
                        .map_err(|e| format!("GetOverlappedResult: {e}"))?;
                    n
                } else if wait == WAIT_TIMEOUT {
                    // 取消未完成的读并等待它真正结束（见模块文档）。
                    unsafe {
                        let _ = CancelIoEx(handle, Some(&overlapped as *const OVERLAPPED));
                        let mut n = 0u32;
                        let _ = GetOverlappedResult(handle, &overlapped, &mut n, true);
                        let _ = windows::Win32::Foundation::CloseHandle(event);
                    }
                    return Ok(None);
                } else {
                    unsafe {
                        let _ = windows::Win32::Foundation::CloseHandle(event);
                    }
                    return Err("等待 HID 输入报告失败".to_string());
                }
            } else {
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(event);
                }
                return Err(format!("ReadFile: {e}"));
            }
        }
    };
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(event);
    }
    Ok(Some(buf[..(transferred as usize).min(buf.len())].to_vec()))
}

/// 便捷封装：等待一条报告的时长常量（50ms 轮询步长，与 easyop 一致）。
pub const POLL_STEP_MS: u32 = 50;

/// 用默认步长读一条报告。
pub fn read_once_default(handle: HANDLE) -> Result<Option<Vec<u8>>, String> {
    read_input_report_once(handle, Duration::from_millis(POLL_STEP_MS as u64).as_millis() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_step_is_sane() {
        assert!(POLL_STEP_MS >= 10 && POLL_STEP_MS <= 200);
    }
}
