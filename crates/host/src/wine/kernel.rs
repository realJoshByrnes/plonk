//! Kernel-side NT services beyond files/memory: handles, timers, tokens, thread/process info,
//! virtual-memory helpers, keyed events, volume info, more system-information classes.
use super::*;

const STATUS_INFO_LENGTH_MISMATCH: u32 = 0xC000_0004;
const STATUS_INVALID_INFO_CLASS: u32 = 0xC000_0003;
const STATUS_NOT_SUPPORTED: u32 = 0xC000_00BB;
const STATUS_TIMEOUT: u32 = 0x102;

impl WineProc {
    fn is_pseudo(h: u32) -> bool { h == u32::MAX || h == u32::MAX - 1 || h == u32::MAX - 3 || h == u32::MAX - 4 || h == 0xFFFF_FFFA }

    fn put_sid(&mut self, at: u32) -> Res<u32> {
        // S-1-5-21-0-0-0-1000
        self.mem.write_bytes(at, &[1, 5, 0, 0, 0, 0, 0, 5]).map_err(|_| "sid")?;
        for (i, v) in [21u32, 0, 0, 0, 1000].iter().enumerate() { self.w32(at + 8 + i as u32 * 4, *v)?; }
        Ok(28)
    }

    pub(super) fn kernel_call(&mut self, name: &str, a: &[u32]) -> Res<Option<u32>> {
        Ok(Some(match name {
            "NtDuplicateObject" => {
                // (src_proc, src_handle, dst_proc, *dst_handle, access, attrs, options)
                let h = a[1];
                let nh = if Self::is_pseudo(h) { h } else {
                    let Some(o) = self.handles.get(&h).cloned() else { return Ok(Some(STATUS_INVALID_HANDLE)) };
                    self.new_handle(o)
                };
                if a[3] != 0 { self.w32(a[3], nh)?; }
                if a[6] & 1 != 0 && !Self::is_pseudo(h) { self.handles.remove(&h); } // DUPLICATE_CLOSE_SOURCE
                0
            }
            "NtQueryObject" => {
                // (handle, class, buf, len, retlen)
                if a[1] == 0 { // ObjectBasicInformation (0x38 bytes)
                    if a[3] < 0x38 { return Ok(Some(STATUS_INFO_LENGTH_MISMATCH)); }
                    for o in (0..0x38).step_by(4) { self.w32(a[2] + o, 0)?; }
                    self.w32(a[2] + 8, 0x1F_01FF)?; self.w32(a[2] + 12, 1)?; self.w32(a[2] + 16, 1)?;
                    if a[4] != 0 { self.w32(a[4], 0x38)?; }
                    0
                } else { self.note(format!("QueryObject class {}", a[1])); STATUS_INVALID_INFO_CLASS }
            }
            "NtOpenEvent" | "NtOpenMutant" | "NtOpenSemaphore" | "NtOpenTimer" | "NtOpenSection" | "NtOpenKeyedEvent" => STATUS_OBJECT_NAME_NOT_FOUND,
            "NtCreateTimer" => { let h = self.new_handle(Obj::Sync { signaled: false, auto: a[3] == 1, count: 0 }); self.w32(a[0], h)?; 0 }
            "NtSetTimer" => {
                // (h, due*, apc, apc_ctx, resume, period_ms, prev_state*)
                let t = self.r32(a[1])? as u64 | (self.r32(a[1] + 4)? as u64) << 32;
                let due = if (t as i64) < 0 { self.wm_now_ms() + (-(t as i64)) as u64 / 10_000 } else { 0 };
                self.ktimers.push((a[0], due, a[5]));
                if let Some(Obj::Sync { signaled, .. }) = self.handles.get_mut(&a[0]) { *signaled = false; }
                if a[6] != 0 { self.w32(a[6], 0)?; }
                0
            }
            "NtCancelTimer" => { self.ktimers.retain(|t| t.0 != a[0]); if a[1] != 0 { self.w32(a[1], 0)?; } 0 }
            "NtQueryTimerResolution" => { self.w32(a[0], 156_250)?; self.w32(a[1], 5_000)?; self.w32(a[2], 10_000)?; 0 }
            "NtSetTimerResolution" => { if a[3] != 0 { self.w32(a[3], 10_000)?; } 0 }
            "NtGetCurrentProcessorNumber" => 0,
            "NtFlushInstructionCache" | "NtLockVirtualMemory" | "NtUnlockVirtualMemory" | "NtFlushVirtualMemory" | "NtSetInformationProcess" | "NtSetInformationObject"
            | "NtCancelIoFile" | "NtCancelIoFileEx" | "NtAdjustPrivilegesToken" | "NtSetInformationVirtualMemory" | "NtYieldExecution_" | "NtSetEaFile" => 0,
            "NtSetInformationThread" => 0,
            "NtQueryInformationThread" => {
                // (h, class, buf, len, retlen); class 0 = ThreadBasicInformation
                match a[1] {
                    0 => {
                        if a[3] < 28 { return Ok(Some(STATUS_INFO_LENGTH_MISMATCH)); }
                        for (i, v) in [0u32, TEB, 0x20, 0x24, 1, 8, 0].iter().enumerate() { self.w32(a[2] + i as u32 * 4, *v)?; }
                        if a[4] != 0 { self.w32(a[4], 28)?; }
                        0
                    }
                    14 | 9 | 12 => { if a[3] >= 4 { self.w32(a[2], 0)?; } if a[4] != 0 { self.w32(a[4], 4)?; } 0 } // GetThreadIOPendingFlag / ThreadIsIoPending
                    _ => { self.note(format!("QueryInformationThread class {}", a[1])); STATUS_INVALID_INFO_CLASS }
                }
            }
            "NtOpenProcessToken" | "NtOpenProcessTokenEx" | "NtOpenThreadToken" | "NtOpenThreadTokenEx" | "NtDuplicateToken" => {
                let out = match name { "NtOpenProcessToken" => a[2], "NtOpenProcessTokenEx" => a[3], "NtOpenThreadToken" => a[3], "NtOpenThreadTokenEx" => a[4], _ => a[6] };
                if name.starts_with("NtOpenThreadToken") { return Ok(Some(0xC000_007C)); } // STATUS_NO_TOKEN: not impersonating
                let h = self.new_handle(Obj::Sync { signaled: true, auto: false, count: 0xEE }); // token marker
                self.w32(out, h)?;
                0
            }
            "NtQueryInformationToken" if a[1] != 1 => {
                let (class, buf, len, ret) = (a[1], a[2], a[3], a[4]);
                let need: u32 = match class { 2 => 4 + 8 + 28, 3 => 4, 4 | 5 => 4 + 28, 8 | 9 | 12 | 18 | 20 | 21 | 25 | 26 => 4, 6 => 8, 10 => 56, _ => { self.note(format!("QueryInformationToken class {class}")); return Ok(Some(STATUS_INVALID_INFO_CLASS)); } };
                if ret != 0 { self.w32(ret, need)?; }
                if len < need { return Ok(Some(STATUS_INFO_LENGTH_MISMATCH)); }
                for o in (0..need).step_by(4) { self.w32(buf + o, 0)?; }
                match class {
                    2 => { self.w32(buf, 1)?; self.w32(buf + 4, buf + 12)?; self.w32(buf + 8, 7)?; self.put_sid(buf + 12)?; }
                    4 | 5 => { self.w32(buf, buf + 4)?; self.put_sid(buf + 4)?; }
                    8 => self.w32(buf, 1)?,            // TokenPrimary
                    18 => self.w32(buf, 1)?,           // TokenElevationType: default
                    20 => self.w32(buf, 1)?,           // TokenElevation: elevated
                    _ => {}
                }
                0
            }
            "NtReadVirtualMemory" | "NtWriteVirtualMemory" => {
                // (process, base, buf, size, *done): self-process only
                let (base, buf, n) = if name == "NtReadVirtualMemory" { (a[1], a[2], a[3]) } else { (a[2], a[1], a[3]) };
                let data = self.mem.read_bytes(base, n).map_err(|_| "readvm")?.to_vec();
                if name == "NtReadVirtualMemory" { self.mem.write_bytes(buf, &data).map_err(|_| "readvm")?; } else { self.mem.write_bytes(a[1], &self.mem.read_bytes(a[2], n).map_err(|_| "writevm")?.to_vec()).map_err(|_| "writevm")?; }
                if a[4] != 0 { self.w32(a[4], n)?; }
                0
            }
            "NtCreateKeyedEvent" => { let h = self.new_handle(Obj::Sync { signaled: false, auto: true, count: 0 }); self.w32(a[0], h)?; 0 }
            "NtReleaseKeyedEvent" => 0,
            "NtWaitForKeyedEvent" | "NtWaitForAlertByThreadId" => { self.idle = true; STATUS_TIMEOUT }
            "NtAlertThreadByThreadId" | "NtAlertThread" => 0,
            "NtOpenDirectoryObject" => { let h = self.new_handle(Obj::Key(String::from("objdir"))); self.w32(a[0], h)?; 0 }
            "NtQueryDirectoryObject" => {
                // (h, buf, len, single, restart, *ctx, *retlen): the object directory lists one drive symlink, C:
                let ctx = self.r32(a[5])?;
                if ctx > 0 && a[4] == 0 { return Ok(Some(0x8000_001A)); }
                let (name, ty) = ("C:", "SymbolicLink");
                let need = 16 + (name.len() as u32 + ty.len() as u32 + 2) * 2 + 16;
                if a[2] < need { return Ok(Some(0x8000_0005)); }
                let (n0, t0) = (a[1] + 32, a[1] + 32 + (name.len() as u32 + 1) * 2);
                for (i, c) in name.encode_utf16().enumerate() { self.w16(n0 + i as u32 * 2, c)?; } self.w16(n0 + name.len() as u32 * 2, 0)?;
                for (i, c) in ty.encode_utf16().enumerate() { self.w16(t0 + i as u32 * 2, c)?; } self.w16(t0 + ty.len() as u32 * 2, 0)?;
                for (o, v) in [(0u32, (name.len() as u32 * 2) | ((name.len() as u32 * 2 + 2) << 16)), (4, n0), (8, (ty.len() as u32 * 2) | ((ty.len() as u32 * 2 + 2) << 16)), (12, t0), (16, 0), (20, 0), (24, 0), (28, 0)] { self.w32(a[1] + o, v)?; }
                self.w32(a[5], 1)?;
                if a[6] != 0 { self.w32(a[6], need)?; }
                0
            }
            "NtOpenSymbolicLinkObject" => { let h = self.new_handle(Obj::Key(String::from("symlink"))); self.w32(a[0], h)?; 0 }
            "NtQuerySymbolicLinkObject" => {
                let t = "\\Device\\HarddiskVolume1";
                let (max, buf) = (self.mem.r16(a[1] + 2).unwrap_or(0) as u32, self.r32(a[1] + 4)?);
                if max < t.len() as u32 * 2 { return Ok(Some(0xC000_0023)); }
                for (i, c) in t.encode_utf16().enumerate() { self.w16(buf + i as u32 * 2, c)?; }
                self.mem.w16(a[1], t.len() as u16 * 2).ok();
                if a[2] != 0 { self.w32(a[2], t.len() as u32 * 2)?; }
                0
            }
            "NtDeviceIoControlFile" | "NtFsControlFile_" | "NtCreateNamedPipeFile" | "NtCreateMailslotFile" | "NtCreateIoCompletion" | "NtSetIoCompletion" | "NtRemoveIoCompletion" | "NtQueryEaFile" => STATUS_NOT_SUPPORTED,
            "NtQueryVolumeInformationFile" if matches!(a[4], 1 | 3 | 5 | 7) => {
                let (iosb, buf, len, class) = (a[1], a[2], a[3], a[4]);
                let need = match class { 1 => 20 + 2, 3 => 24, 5 => 12 + 8, _ => 32 };
                if len < need { return Ok(Some(STATUS_INFO_LENGTH_MISMATCH)); }
                for o in (0..need).step_by(4) { self.w32(buf + o, 0)?; }
                match class {
                    1 => { self.w32(buf + 8, 0x1234_5678)?; } // volume serial
                    3 | 7 => { self.w32(buf, 0x0100_0000)?; self.w32(buf + 8, 0x00C0_0000)?; if class == 7 { self.w32(buf + 16, 0x00C0_0000)?; self.w32(buf + 24, 8)?; self.w32(buf + 28, 512)?; } else { self.w32(buf + 16, 8)?; self.w32(buf + 20, 512)?; } }
                    _ => { self.w32(buf, 0x0000_00FF)?; self.w32(buf + 4, 255)?; self.w32(buf + 8, 8)?; for (i, c) in "NTFS".encode_utf16().enumerate() { self.w16(buf + 12 + i as u32 * 2, c)?; } }
                }
                self.w32(iosb, 0)?; self.w32(iosb + 4, need)?;
                0
            }
            "NtQueryInformationProcess" if matches!(a[1], 3 | 4 | 14 | 23 | 27 | 30 | 34) => {
                let (class, buf, len, ret) = (a[1], a[2], a[3], a[4]);
                let need = match class { 3 => 32, 4 => 48, 14 => 44, 23 => 36, 27 => 8 + 40, 30 => 4, _ => 4 };
                if ret != 0 { self.w32(ret, need)?; }
                if len < need { return Ok(Some(STATUS_INFO_LENGTH_MISMATCH)); }
                for o in (0..need).step_by(4) { self.w32(buf + o, 0)?; }
                match class {
                    23 => { self.w32(buf, 1 << 2)?; self.mem.w8(buf + 4 + 32 + 0, 0).ok(); self.mem.w8(buf + 4 + 2, 3).map_err(|_| "devmap")?; } // C: is a fixed disk
                    27 => { let p = self.exe_path.clone(); let s = format!("\\??\\C:{}", &p[2..]); self.put_ustr(buf, &s); }
                    30 => return Ok(Some(0xC000_0353)), // no debug object
                    34 => self.w32(buf, 0x2)?,         // ProcessExecuteFlags: MEM_EXECUTE_OPTION_ENABLE... DEP permanent bit
                    _ => {}
                }
                0
            }
            "NtQuerySystemInformation" if a[0] == 2 && a[2] >= 0x138 => {
                for o in (0..0x138).step_by(4) { self.w32(a[1] + o, 0)?; }
                for (o, v) in [(0x2Cu32, 0x8000u32), (0x30, 0x4000), (0x34, 0x20000), (0x38, 0x4000)] { self.w32(a[1] + o, v)?; } // available / committed / limit / peak pages
                if a[3] != 0 { self.w32(a[3], 0x138)?; }
                0
            }
            "NtQuerySystemInformation" if matches!(a[0], 2 | 8 | 11 | 35 | 0x15 | 0x17 | 0x2c | 0x49 | 0x5e) => {
                let (class, buf, len, ret) = (a[0], a[1], a[2], a[3]);
                let need = match class { 2 => 0x138, 8 => 0x30, 11 => 4 + 0x120, 35 => 2, 0x15 => 0x24, 0x17 => 0x18, _ => 0x40 };
                if ret != 0 { self.w32(ret, need)?; }
                if len < need { return Ok(Some(STATUS_INFO_LENGTH_MISMATCH)); }
                for o in (0..need).step_by(4) { self.w32(buf + o, 0)?; }
                0
            }
            _ => return Ok(None),
        }))
    }

    /// Signal kernel timers that came due (called from the shell tick).
    pub(crate) fn fire_ktimers(&mut self) {
        let now = self.wm_now_ms();
        let mut again = Vec::new();
        for (h, due, period) in std::mem::take(&mut self.ktimers) {
            if due <= now {
                if let Some(Obj::Sync { signaled, .. }) = self.handles.get_mut(&h) { *signaled = true; }
                if period > 0 { again.push((h, now + period as u64, period)); }
            } else { again.push((h, due, period)); }
        }
        self.ktimers = again;
    }
}
