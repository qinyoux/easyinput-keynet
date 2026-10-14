//! Win32 HID 设备枚举与句柄管理（自 easyop `device.rs` 原样移植，2026-10-14）。
//!
//! EasyInput V2.0 是「单 HID 接口 + 多顶层集合」：键盘集合在最前、无 Feature
//! 报告，`HidP_GetCaps` 只返回首集合 caps，导致 hidapi 内部 Feature/Input 缓冲
//! 按键盘集合取值而失效（HidD_SetFeature 收到 0 长度 → ERROR_INVALID_FUNCTION）。
//! 因此这里绕过 hidapi，直接用 Win32 HID API（SetupDi 枚举 + CreateFileW +
//! HidD_GetAttributes）。
//!
//! Companion 只用**读输入报告**（0x11 kind 0x05）这一半能力，不下发配置
//! （配置走 easyop），因此移植时去掉了 send_config / send_command / 0x10 分块等
//! 下发路径，保留枚举、VID/PID 校验、Feature Report 集合判别与句柄关闭。

use std::mem::size_of;
use std::slice;

use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
};
use windows::Win32::Devices::HumanInterfaceDevice::{HidD_GetAttributes, HidD_GetHidGuid, HIDD_ATTRIBUTES};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, HWND};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

use crate::constants::{PID, VID};

/// 枚举当前所有 HID 设备接口路径（SetupDi）。
pub fn enumerate_device_paths() -> Vec<String> {
    let guid = unsafe { HidD_GetHidGuid() };
    let mut paths = Vec::new();
    let dev_info = match unsafe {
        SetupDiGetClassDevsW(Some(&guid), PCWSTR::null(), HWND::default(), DIGCF_DEVICEINTERFACE | DIGCF_PRESENT)
    } {
        Ok(h) => h,
        Err(_) => return paths,
    };
    let mut index = 0u32;
    loop {
        let mut if_data = SP_DEVICE_INTERFACE_DATA::default();
        if_data.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
        if unsafe { SetupDiEnumDeviceInterfaces(dev_info, None, &guid, index, &mut if_data) }.is_err() {
            break;
        }
        index += 1;
        let mut required = 0u32;
        let _ = unsafe {
            SetupDiGetDeviceInterfaceDetailW(dev_info, &if_data, None, 0, Some(&mut required), None)
        };
        if required == 0 {
            continue;
        }
        let mut buffer = vec![0u8; required as usize];
        let detail_ptr = buffer.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
        unsafe {
            (*detail_ptr).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
        }
        if unsafe {
            SetupDiGetDeviceInterfaceDetailW(dev_info, &if_data, Some(detail_ptr), required, None, None)
        }
        .is_err()
        {
            continue;
        }
        let path_ptr = unsafe { (*detail_ptr).DevicePath.as_ptr() };
        let mut len = 0usize;
        while unsafe { *path_ptr.add(len) } != 0 {
            len += 1;
        }
        let path = unsafe { String::from_utf16_lossy(slice::from_raw_parts(path_ptr, len)) };
        if !path.is_empty() {
            paths.push(path);
        }
    }
    unsafe {
        let _ = SetupDiDestroyDeviceInfoList(dev_info);
    }
    paths
}

/// 打开路径并校验 VID/PID，匹配则返回句柄。
pub fn open_matching(path: &str) -> Option<HANDLE> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            HANDLE::default(),
        )
    }
    .ok()?;
    let mut attrs = HIDD_ATTRIBUTES::default();
    attrs.Size = size_of::<HIDD_ATTRIBUTES>() as u32;
    let ok = unsafe { HidD_GetAttributes(handle, &mut attrs) };
    if !ok.as_bool() || attrs.VendorID != VID || attrs.ProductID != PID {
        unsafe {
            let _ = CloseHandle(handle);
        }
        return None;
    }
    Some(handle)
}

/// 检测开发板 HID 是否在线，返回设备描述。
pub fn device_status() -> Result<String, String> {
    for path in enumerate_device_paths() {
        if let Some(handle) = open_matching(&path) {
            close_handle(handle);
            return Ok(format!("EasyInput AI ({VID:04X}:{PID:04X})"));
        }
    }
    Err(format!("未找到开发板（VID {VID:04X} / PID {PID:04X}），请确认已通过 USB 连接"))
}

/// 判断一个已打开的 HID 句柄是否支持 Feature Report（区分 app-command 集合与键盘集合）。
///
/// 键盘集合（col01）无 Feature Report，app-command 集合（col02）有 Feature Report。
/// 打开键盘集合会干扰系统键盘事件路由，监听器应仅打开支持 Feature Report 的集合。
pub fn has_feature_report(handle: HANDLE) -> bool {
    use windows::Win32::Devices::HumanInterfaceDevice::{
        HidD_FreePreparsedData, HidD_GetPreparsedData, HidP_GetCaps, HIDP_CAPS, HIDP_STATUS_SUCCESS,
        PHIDP_PREPARSED_DATA,
    };
    let mut preparsed = PHIDP_PREPARSED_DATA::default();
    let ok = unsafe { HidD_GetPreparsedData(handle, &mut preparsed) };
    if !ok.as_bool() {
        return false;
    }
    let mut caps = HIDP_CAPS::default();
    let status = unsafe { HidP_GetCaps(preparsed, &mut caps) };
    unsafe { HidD_FreePreparsedData(preparsed) };
    if status != HIDP_STATUS_SUCCESS {
        return false;
    }
    caps.NumberFeatureValueCaps > 0 || caps.NumberFeatureButtonCaps > 0
}

/// 关闭句柄（吞掉错误）。
pub fn close_handle(handle: HANDLE) {
    unsafe {
        let _ = CloseHandle(handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerate_returns_vec() {
        // 仅验证接口可调用（可能为空），不应 panic。
        let _ = enumerate_device_paths();
    }

    #[test]
    fn device_status_does_not_panic_without_board() {
        // 无板环境返回 Err 字符串；有板环境返回 Ok。两种都不应 panic。
        let _ = device_status();
    }
}
