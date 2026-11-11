//! clipboard, keyboard layouts, display/monitor/DPI enumeration, window stations/desktops, raw input, IME stubs, hotkeys (win32u calls filled in systematically; see docs/WIN32U_COVERAGE.txt).
use super::*;
use std::collections::BTreeMap;

const WINSTA: u32 = 0x0E00_0001;
const DESKTOP_H: u32 = 0x0E00_0002;
const MONITOR: u32 = 0x20001;
const HKL: u32 = 0x0409_0409;
const NEG1: u32 = 0xFFFF_FFFF;
const ADAPTER: &str = "\\\\.\\DISPLAY1";

/// Per-area state (add fields here; `Extra` owns one of these).
#[derive(Default)]
pub struct State {
    listeners: Vec<u32>,
    viewer: u32,
    seq: u32,
    seq_seen: Option<(u32, usize)>,
    winsta: u32,
    desktop: u32,
    shell: (u32, u32),
    raw_devs: Vec<[u32; 3]>,
    mouse_in_pointer: bool,
    next_himc: u32,
    hotkeys: BTreeMap<(u32, i32), (u32, u32)>,
}

const MODES: [(u32, u32); 5] = [(640, 480), (800, 600), (1024, 768), (1280, 1024), (1920, 1080)];

impl WineProc {
    fn put_w(&mut self, at: u32, s: &str, max: u32) -> Res<()> {
        let u: Vec<u16> = s.encode_utf16().take(max as usize - 1).collect();
        for (i, c) in u.iter().enumerate() { self.mem.w16(at + i as u32 * 2, *c).map_err(|_| "misc w16")?; }
        self.mem.w16(at + u.len() as u32 * 2, 0).map_err(|_| "misc w16")?;
        Ok(())
    }

    fn fill_devmode(&mut self, p: u32, w: u32, h: u32) -> Res<()> {
        for o in (0..220).step_by(4) { self.w32(p + o, 0)?; }
        self.put_w(p, ADAPTER, 32)?;
        self.mem.w16(p + 64, 0x0401).map_err(|_| "dm")?;
        self.mem.w16(p + 68, 220).map_err(|_| "dm")?;
        self.w32(p + 72, 0x0020 | 0x0080 | 0x4_0000 | 0x8_0000 | 0x10_0000 | 0x20_0000 | 0x40_0000)?;
        self.w32(p + 168, 32)?;
        self.w32(p + 172, w)?;
        self.w32(p + 176, h)?;
        self.w32(p + 184, 60)?;
        Ok(())
    }

    /// Ok(None) = not handled here (falls through to the next handler / the lenient stub).
    pub(super) fn misc_batch_call(&mut self, name: &str, a: &[u32]) -> Res<Option<u32>> {
        let (sw, sh) = (self.wm.screen.0 as u32, self.wm.screen.1 as u32);
        let arg = |i: usize| a.get(i).copied().unwrap_or(0);
        Ok(Some(match name {
            // ---- clipboard ----
            "NtUserAddClipboardFormatListener" => { let h = arg(0); let l = &mut self.wm.extra.misc_b.listeners; if !l.contains(&h) { l.push(h); } 1 }
            "NtUserRemoveClipboardFormatListener" => {
                let h = arg(0); let l = &mut self.wm.extra.misc_b.listeners;
                let n = l.len(); l.retain(|x| *x != h); (l.len() != n) as u32
            }
            "NtUserSetClipboardViewer" => { let s = &mut self.wm.extra.misc_b; let old = s.viewer; s.viewer = arg(0); old }
            "NtUserGetClipboardViewer" => self.wm.extra.misc_b.viewer,
            "NtUserChangeClipboardChain" => { let s = &mut self.wm.extra.misc_b; if s.viewer == arg(0) { s.viewer = arg(1); } 1 }
            "NtUserCountClipboardFormats" => self.wm.extra.clip_text.is_some() as u32,
            "NtUserGetClipboardOwner" | "NtUserGetOpenClipboardWindow" => 0,
            "NtUserGetClipboardSequenceNumber" => {
                let cur = (self.wm.extra.clip_fmt, self.wm.extra.clip_text.as_ref().map_or(usize::MAX, |t| t.len()));
                let s = &mut self.wm.extra.misc_b;
                if s.seq_seen != Some(cur) { s.seq_seen = Some(cur); s.seq += 1; }
                s.seq
            }
            "NtUserGetClipboardFormatName" => 0, // registered formats live in the atom table, which win32u does not own here
            "NtUserGetPriorityClipboardFormat" => {
                if self.wm.extra.clip_text.is_none() { return Ok(Some(0)); }
                let fmt = self.wm.extra.clip_fmt;
                let mut r = NEG1;
                for i in 0..arg(1) { if self.r32(arg(0) + i * 4)? == fmt { r = fmt; break; } }
                r
            }
            "NtUserGetUpdatedClipboardFormats" => {
                // (formats*, size, out_size*)
                if arg(2) == 0 { return Ok(Some(0)); }
                let n = self.wm.extra.clip_text.is_some() as u32;
                self.w32(arg(2), n)?;
                if arg(1) < n { return Ok(Some(0)); }
                if n == 1 { let f = self.wm.extra.clip_fmt; self.w32(arg(0), f)?; }
                1
            }

            // ---- keyboard layouts ----
            "NtUserActivateKeyboardLayout" => HKL,
            "NtUserGetKeyboardLayoutList" => {
                if arg(0) != 0 && arg(1) != 0 { self.w32(arg(1), HKL)?; }
                1
            }
            "NtUserGetKeyboardLayoutName" => { self.put_w(arg(0), "00000409", 9)?; 1 }
            "NtUserBuildHimcList" => { if arg(3) != 0 { self.w32(arg(3), 0)?; } 0 }

            // ---- display ----
            "NtUserEnumDisplayDevices" => {
                // (device UNICODE_STRING* or NULL, index, DISPLAY_DEVICEW*, flags): NULL = adapters, else that adapter's monitors
                if arg(1) != 0 || arg(2) == 0 { return Ok(Some(0xC000_0001)); }
                let d = arg(2);
                let adapters = arg(0) == 0 || self.r32(arg(0) + 4)? == 0;
                let (nm, ds, fl) = if adapters { (ADAPTER.to_string(), "Plonk Display Adapter", 5u32) } else { (format!("{ADAPTER}\\Monitor0"), "Generic PnP Monitor", 3u32) };
                self.put_w(d + 4, &nm, 32)?;
                self.put_w(d + 68, ds, 128)?;
                self.w32(d + 324, fl)?;
                self.put_w(d + 328, if adapters { "PCI\\VEN_0000&DEV_0000" } else { "MONITOR\\PLK0001" }, 128)?;
                self.put_w(d + 584, "", 128)?;
                0
            }
            "NtUserEnumDisplaySettings" => {
                // (device*, mode, DEVMODEW*, flags)
                let m = arg(1) as i32;
                let (w, h) = if m < 0 { (sw, sh) } else {
                    let mut list: Vec<(u32, u32)> = MODES.to_vec();
                    if !list.contains(&(sw, sh)) { list.push((sw, sh)); list.sort(); }
                    match list.get(m as usize) { Some(v) => *v, None => return Ok(Some(0)) }
                };
                self.fill_devmode(arg(2), w, h)?;
                1
            }
            "NtUserChangeDisplaySettings" => 0, // DISP_CHANGE_SUCCESSFUL; the surface keeps its size (resizing the live desktop is not safe)
            "NtUserEnumDisplayMonitors" => {
                // (hdc, RECT* clip, MONITORENUMPROC, lparam)
                let mut r = [0, 0, sw as i32, sh as i32];
                if arg(1) != 0 {
                    let c = self.rect_at(arg(1))?;
                    r = [r[0].max(c[0]), r[1].max(c[1]), r[2].min(c[2]), r[3].min(c[3])];
                    if r[0] >= r[2] || r[1] >= r[3] { return Ok(Some(1)); }
                }
                if arg(2) == 0 { return Ok(Some(1)); }
                // scratch RECT in the free area call_guest reserves below ESP
                let at = self.cpu.r[ESP].wrapping_sub(0x100);
                self.put_rect(at, r)?;
                self.call_guest(arg(2), &[MONITOR, arg(0), at, arg(3)])?;
                1
            }
            "NtUserDisplayConfigGetDeviceInfo" => {
                let p = arg(0);
                match self.r32(p)? {
                    1 => { self.put_w(p + 20, ADAPTER, 32)?; 0 }
                    2 => {
                        self.w32(p + 20, 0)?; self.w32(p + 24, NEG1)?; self.w32(p + 28, 0)?; self.w32(p + 32, 0)?;
                        self.put_w(p + 36, "Generic PnP Monitor", 64)?; self.put_w(p + 164, "\\\\?\\DISPLAY#PLK0001#1", 128)?;
                        0
                    }
                    4 => { self.put_w(p + 20, "\\\\?\\PCI#VEN_0000&DEV_0000", 128)?; 0 }
                    _ => 87,
                }
            }
            "NtUserGetDisplayConfigBufferSizes" => {
                if arg(1) != 0 { self.w32(arg(1), 1)?; }
                if arg(2) != 0 { self.w32(arg(2), 2)?; }
                0
            }
            "NtUserQueryDisplayConfig" => {
                // (flags, paths_count*, paths*, modes_count*, modes*, topology*)
                let (pc, mc) = (arg(1), arg(3));
                if pc == 0 || mc == 0 || arg(2) == 0 || arg(4) == 0 { return Ok(Some(87)); }
                if self.r32(pc)? < 1 || self.r32(mc)? < 2 { return Ok(Some(122)); }
                let (p, m) = (arg(2), arg(4));
                for o in (0..72).step_by(4) { self.w32(p + o, 0)?; }
                for o in (0..128).step_by(4) { self.w32(m + o, 0)?; }
                // path: source{luid, id, modeIdx, status}, target{luid, id, modeIdx, tech, rotation, scaling, refresh n/d, scanline, available, status}, flags
                self.w32(p + 16, 1)?;
                self.w32(p + 20 + 12, 1)?; // target modeInfoIdx = 1
                self.w32(p + 20 + 16, NEG1)?; self.w32(p + 20 + 20, 1)?; self.w32(p + 20 + 24, 1)?;
                self.w32(p + 20 + 28, 60)?; self.w32(p + 20 + 32, 1)?; self.w32(p + 20 + 36, 1)?; self.w32(p + 20 + 40, 1)?; self.w32(p + 20 + 44, 1)?;
                self.w32(p + 68, 1)?;
                // mode 0: source (infoType 1); mode 1: target (infoType 2)
                self.w32(m, 1)?; self.w32(m + 16, sw)?; self.w32(m + 20, sh)?; self.w32(m + 24, 4)?;
                let t = m + 64;
                self.w32(t, 2)?;
                self.w32(t + 16, sw * sh * 60)?;
                self.w32(t + 32, 60)?; self.w32(t + 36, 1)?; // vSyncFreq
                self.w32(t + 16 + 24, sw)?; self.w32(t + 16 + 28, sh)?; // activeSize
                self.w32(t + 16 + 32, sw)?; self.w32(t + 16 + 36, sh)?; // totalSize
                self.w32(t + 16 + 44, 1)?;
                self.w32(pc, 1)?; self.w32(mc, 2)?;
                0
            }

            // ---- DPI ----
            "NtUserGetDpiForMonitor" => { if arg(2) != 0 { self.w32(arg(2), 96)?; } if arg(3) != 0 { self.w32(arg(3), 96)?; } 1 }
            "NtUserSetProcessDpiAwarenessContext" => 1,
            "NtUserLogicalToPerMonitorDPIPhysicalPoint" | "NtUserPerMonitorDPIPhysicalToLogicalPoint" => 1,
            "NtUserSystemParametersInfoForDpi" => {
                // (action, val, ptr, winini, dpi): same as SystemParametersInfo; everything is 96 dpi
                return self.win32u_call("NtUserSystemParametersInfo", &[arg(0), arg(1), arg(2), arg(3)]);
            }

            // ---- window stations / desktops ----
            "NtUserCreateWindowStation" | "NtUserOpenWindowStation" => WINSTA,
            "NtUserCloseWindowStation" | "NtUserCloseDesktop" | "NtUserSetObjectInformation" => 1,
            "NtUserGetProcessWindowStation" => { let s = self.wm.extra.misc_b.winsta; if s == 0 { WINSTA } else { s } }
            "NtUserSetProcessWindowStation" => { self.wm.extra.misc_b.winsta = arg(0); 1 }
            "NtUserCreateDesktopEx" | "NtUserOpenDesktop" | "NtUserOpenInputDesktop" => DESKTOP_H,
            "NtUserGetThreadDesktop" => { let s = self.wm.extra.misc_b.desktop; if s == 0 { DESKTOP_H } else { s } }
            "NtUserSetThreadDesktop" => { self.wm.extra.misc_b.desktop = arg(0); 1 }
            "NtUserSetShellWindowEx" => { self.wm.extra.misc_b.shell = (arg(0), arg(1)); 1 }

            // ---- raw input / pointer ----
            "NtUserRegisterRawInputDevices" => {
                // (devices*, count, size): RAWINPUTDEVICE = usage page/usage (2x u16), flags, hwnd
                if arg(2) != 12 || arg(0) == 0 { return Ok(Some(0)); }
                for i in 0..arg(1) {
                    let b = arg(0) + i * 12;
                    let d = [self.r32(b)?, self.r32(b + 4)?, self.r32(b + 8)?];
                    let s = &mut self.wm.extra.misc_b.raw_devs;
                    s.retain(|e| e[0] != d[0]);
                    if d[1] & 1 == 0 { s.push(d); } // RIDEV_REMOVE
                }
                1
            }
            "NtUserGetRegisteredRawInputDevices" => {
                // (devices*, count*, size)
                if arg(2) != 12 || arg(1) == 0 { return Ok(Some(NEG1)); }
                let devs = self.wm.extra.misc_b.raw_devs.clone();
                let n = devs.len() as u32;
                if arg(0) == 0 { self.w32(arg(1), n)?; return Ok(Some(0)); }
                if self.r32(arg(1))? < n { self.w32(arg(1), n)?; return Ok(Some(NEG1)); }
                self.w32(arg(1), n)?;
                for (i, d) in devs.iter().enumerate() { for k in 0..3 { self.w32(arg(0) + i as u32 * 12 + k as u32 * 4, d[k])?; } }
                n
            }
            "NtUserGetRawInputDeviceList" => {
                if arg(2) != 8 || arg(1) == 0 { return Ok(Some(NEG1)); }
                self.w32(arg(1), 0)?;
                0
            }
            "NtUserGetRawInputDeviceInfo" | "NtUserGetRawInputData" => NEG1,
            "NtUserGetRawInputBuffer" => { if arg(0) == 0 && arg(1) != 0 { self.w32(arg(1), 0)?; } 0 }
            "NtUserEnableMouseInPointer" => { self.wm.extra.misc_b.mouse_in_pointer = arg(0) != 0; 1 }
            "NtUserIsMouseInPointerEnabled" => self.wm.extra.misc_b.mouse_in_pointer as u32,
            "NtUserGetPointerInfoList" => 0,
            "NtUserGetMouseMovePointsEx" => NEG1,

            // ---- IME ----
            "NtUserCreateInputContext" => { let s = &mut self.wm.extra.misc_b; s.next_himc += 1; 0x0F00_0000 + s.next_himc }
            "NtUserDestroyInputContext" | "NtUserUpdateInputContext" | "NtUserDisableThreadIme" => 1,
            "NtUserAssociateInputContext" | "NtUserNotifyIMEStatus" => 0,

            // ---- hotkeys ----
            "NtUserRegisterHotKey" => {
                let k = (arg(0), arg(1) as i32);
                let t = &mut self.wm.extra.misc_b.hotkeys;
                if t.contains_key(&k) || t.values().any(|v| *v == (arg(2), arg(3))) { return Ok(Some(0)); }
                t.insert(k, (arg(2), arg(3)));
                1
            }
            "NtUserUnregisterHotKey" => self.wm.extra.misc_b.hotkeys.remove(&(arg(0), arg(1) as i32)).is_some() as u32,

            // PrintWindow: no offscreen window rendering path yet; report failure honestly
            "NtUserPrintWindow" => 0,
            _ => return Ok(None),
        }))
    }
}
