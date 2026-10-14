//! CRC-16/CCITT-FALSE：与 EasyInput 固件 `crc16_ccitt` 及 ART-Pi
//! `sh_frame_crc16` 三方一致（init 0xFFFF、poly 0x1021、MSB-first）。
//!
//! 直接计算法（无查表）：250B 帧约 2000 次迭代，PC 上纳秒级，无需为 256×2B
//! 的表付出缓存与初始化成本。ART-Pi 侧同款实现（M7@480MHz 微秒级）。

/// 计算 `crc16_ccitt`。
pub fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_check_value() {
        // CRC-16/CCITT-FALSE 标准校验值：crc16("123456789") == 0x29B1。
        // 这是协议实现的"金标准"测试——任何参数错一位（init/poly/移位方向）
        // 都会在这里现形，且 PC 与 ART-Pi 两侧必须同时过。
        assert_eq!(crc16_ccitt(b"123456789"), 0x29B1);
    }

    #[test]
    fn empty_input_is_init() {
        assert_eq!(crc16_ccitt(b""), 0xFFFF);
    }

    #[test]
    fn deterministic() {
        let data = b"companion-v23-frame";
        assert_eq!(crc16_ccitt(data), crc16_ccitt(data));
    }
}
