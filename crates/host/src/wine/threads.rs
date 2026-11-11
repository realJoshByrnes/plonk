//! Cooperative guest threads. One interpreter, many `Cpu` contexts: a thread keeps running until it blocks
//! (wait / sleep / GetMessage), yields, or exhausts its instruction slice. When every thread is stuck the guest
//! clock jumps to the next deadline (or control returns to the shell).
use super::*;
use plonk_cpu::Cpu;

pub(super) struct Thread {
    pub cpu: Cpu,
    pub tid: u32,
    pub teb: u32,
    pub done: bool,
    pub exit_code: u32,
    pub suspend: u32,
    pub alerted: bool,
    orphans: Vec<Cpu>,
    exc: u32,
    deadline: Option<u64>,
}

/// Result of a syscall handler in this module.
pub(super) enum Tc { Done(u32), Retry, NotMine }

pub(super) enum Stuck { Continue, Idle }

impl WineProc {
    pub(super) fn init_main_thread(&mut self) {
        self.threads = vec![Thread { cpu: Cpu::new(), tid: 0x24, teb: TEB, done: false, exit_code: 0x103, suspend: 0, alerted: false, orphans: Vec::new(), exc: 0, deadline: None }];
        self.cur = 0;
    }

    pub(super) fn current_tid(&self) -> u32 { self.threads.get(self.cur).map_or(0x24, |t| t.tid) }

    fn ready_count(&self) -> usize { self.threads.iter().filter(|t| !t.done && t.suspend == 0).count() }

    fn switch_to(&mut self, k: usize) {
        if k == self.cur { return; }
        let cur = self.cur;
        let (cpu, orphans, exc, dl) = (self.cpu.clone(), std::mem::take(&mut self.orphans), self.exc_depth, self.cur_deadline.take());
        let t = &mut self.threads[cur];
        t.cpu = cpu; t.orphans = orphans; t.exc = exc; t.deadline = dl;
        let n = &mut self.threads[k];
        self.cpu = n.cpu.clone();
        self.orphans = std::mem::take(&mut n.orphans);
        self.exc_depth = n.exc;
        self.cur_deadline = n.deadline.take();
        self.cur = k;
        self.idle = false; self.parked = false; self.took_over = false; self.guest_returned = false;
    }

    /// Switch to the next runnable thread (round robin). Returns false if there is none other than the current one.
    pub(super) fn switch_next(&mut self) -> bool {
        let n = self.threads.len();
        for d in 1..n {
            let k = (self.cur + d) % n;
            if !self.threads[k].done && self.threads[k].suspend == 0 { self.switch_to(k); return true; }
        }
        false
    }

    fn earliest_wake(&self) -> Option<u64> {
        let mut best: Option<u64> = self.cur_deadline;
        let mut note = |t: u64| best = Some(best.map_or(t, |b| b.min(t)));
        for (i, t) in self.threads.iter().enumerate() { if i != self.cur && !t.done && t.suspend == 0 { if let Some(d) = t.deadline { note(d); } } }
        for k in &self.ktimers { note(k.1); }
        if let Some(d) = self.next_wm_timer() { note(d); }
        best
    }

    /// Called by the run loop after a blocking poll. Switches thread, advances the clock, or hands control back.
    pub(super) fn on_stuck(&mut self) -> Stuck {
        let runnable = self.ready_count().max(1);
        if self.no_progress >= runnable {
            self.no_progress = 0;
            let now = self.wm_now_ms();
            return match self.earliest_wake() {
                Some(t) => { self.tick((t.saturating_sub(now)).max(1) as u32); Stuck::Continue }
                None => { self.parked = true; Stuck::Idle }
            };
        }
        if self.cb_depth == 0 { self.switch_next(); }
        Stuck::Continue
    }

    fn timeout_ms(&self, ptr: u32) -> Res<Option<u64>> {
        if ptr == 0 { return Ok(None); }
        let t = self.r32(ptr)? as u64 | (self.r32(ptr + 4)? as u64) << 32;
        Ok(Some(if (t as i64) < 0 { (-(t as i64)) as u64 / 10_000 } else { 0 }))
    }

    /// Retry-style blocking: record the deadline once, time out when it has passed.
    fn wait_or_block(&mut self, tmo_ptr: u32) -> Res<Tc> {
        let now = self.wm_now_ms();
        match self.timeout_ms(tmo_ptr)? {
            Some(0) => { self.cur_deadline = None; return Ok(Tc::Done(0x102)); }
            Some(ms) => { let d = *self.cur_deadline.get_or_insert(now + ms); if now >= d { self.cur_deadline = None; return Ok(Tc::Done(0x102)); } }
            None => {}
        }
        self.blocked = true;
        Ok(Tc::Retry)
    }

    fn signaled(&self, h: u32) -> bool {
        match self.handles.get(&h) {
            Some(Obj::Sync { signaled, count, auto, .. }) => *signaled || (!*auto && *count > 0),
            Some(Obj::Thread(i)) => self.threads.get(*i).is_some_and(|t| t.done),
            _ => false,
        }
    }

    fn consume(&mut self, h: u32) { if let Some(Obj::Sync { signaled, auto, count }) = self.handles.get_mut(&h) { if *auto { *signaled = false; } else if *count > 0 { *count -= 1; if *count == 0 { *signaled = false; } } } }

    fn wait_generic(&mut self, hs: &[u32], any: bool, tmo: u32) -> Res<Tc> {
        let sig: Vec<bool> = hs.iter().map(|h| self.signaled(*h)).collect();
        let ready = if any { sig.iter().position(|s| *s) } else if sig.iter().all(|s| *s) { Some(0) } else { None };
        if let Some(i) = ready {
            if any { self.consume(hs[i]); } else { for h in hs { self.consume(*h); } }
            self.cur_deadline = None;
            return Ok(Tc::Done(i as u32));
        }
        if hs.iter().any(|h| !self.handles.contains_key(h) && !Self::pseudo_ok(*h)) { return Ok(Tc::Done(STATUS_INVALID_HANDLE)); }
        self.wait_or_block(tmo)
    }

    fn pseudo_ok(h: u32) -> bool { h >= 0xFFFF_FFF0 }

    fn new_thread(&mut self, start: u32, arg: u32, suspended: bool, stack_size: u32) -> Res<usize> {
        let tid = 0x28 + 4 * self.threads.len() as u32;
        let teb = self.vm_reserve(None, 0x2000).ok_or("thread: no VM for TEB")?;
        let ssz = if stack_size == 0 { STACK_SIZE } else { align(stack_size, 0x10000).max(0x10000) };
        let stack = self.vm_reserve(None, ssz).ok_or("thread: no VM for stack")?;
        let top = stack + ssz;
        for (o, v) in [(0u32, 0xFFFF_FFFFu32), (4, top), (8, stack), (0x18, teb), (0x20, 0x20), (0x24, tid), (0x30, PEB), (0xC0, SYSCALL_THUNK), (0xE0C, stack)] { self.w32(teb + o, v)?; }
        let acs = self.scratch_alloc(0x20);
        self.w32(teb + 0x1A8, acs)?;
        self.w16(teb + 0xBF8 + 2, 0x20A)?;
        self.w32(teb + 0xBF8 + 4, teb + 0xC00)?;
        // initial CONTEXT -> RtlUserThreadStart(entry in eax, arg in ebx), entered through LdrInitializeThunk
        let esp0 = top - 16;
        let ctx = (esp0 & !3) - 0x2CC;
        self.w32(ctx, 0x1_0007)?;
        for (off, v) in [(0x8Cu32, 0x2Bu32), (0x90, 0x53), (0x94, 0x2B), (0x98, 0x2B), (0xA4, arg), (0xB0, start), (0xB8, self.rtl_user_thread_start), (0xBC, 0x23), (0xC0, 0x202), (0xC4, esp0), (0xC8, 0x2B)] { self.w32(ctx + off, v)?; }
        let mut sp = ctx;
        for v in [0, 0, 0, ctx, 0xdead_babe] { sp -= 4; self.w32(sp, v)?; }
        let mut cpu = Cpu::new();
        cpu.thunk = self.cpu.thunk;
        cpu.fs_base = teb;
        cpu.r[ESP] = sp;
        cpu.eip = self.ldr_initialize_thunk;
        self.threads.push(Thread { cpu, tid, teb, done: false, exit_code: 0x103, suspend: suspended as u32, alerted: false, orphans: Vec::new(), exc: 0, deadline: None });
        Ok(self.threads.len() - 1)
    }

    fn thread_of(&self, h: u32) -> Option<usize> {
        match h { 0 | 0xFFFF_FFFE => Some(self.cur), _ => match self.handles.get(&h) { Some(Obj::Thread(i)) => Some(*i), _ => None } }
    }

    pub(super) fn thread_call(&mut self, name: &str, a: &[u32]) -> Res<Tc> {
        Ok(match name {
            "NtWaitForSingleObject" => self.wait_generic(&[a[0]], true, a[2])?,
            "NtWaitForMultipleObjects" => { let hs: Vec<u32> = (0..a[0]).map(|i| self.r32(a[1] + i * 4).unwrap_or(0)).collect(); self.wait_generic(&hs, a[2] == 1, a[4])? }
            "NtSignalAndWaitForSingleObject" => {
                if let Some(Obj::Sync { signaled, .. }) = self.handles.get_mut(&a[0]) { *signaled = true; }
                self.wait_generic(&[a[1]], true, a[3])?
            }
            "NtDelayExecution" => {
                let ms = self.timeout_ms(a[1])?.unwrap_or(0);
                if ms == 0 { // Sleep(0): give other threads a turn
                    if self.threads.len() > 1 && self.cb_depth == 0 { self.switch_next(); return Ok(Tc::Done(0)); }
                    self.idle = true; return Ok(Tc::Done(0));
                }
                let now = self.wm_now_ms();
                let d = *self.cur_deadline.get_or_insert(now + ms);
                if now >= d { self.cur_deadline = None; Tc::Done(0) } else { self.blocked = true; Tc::Retry }
            }
            "NtYieldExecution" => { if self.threads.len() > 1 && self.cb_depth == 0 { self.switch_next(); } else { self.idle = true; } Tc::Done(0) }
            "NtWaitForKeyedEvent" => { // (h, key, alertable, timeout*)
                if self.keyed.remove(&a[1]) { self.cur_deadline = None; Tc::Done(0) } else { self.wait_or_block(a[3])? }
            }
            "NtReleaseKeyedEvent" => { self.keyed.insert(a[1]); Tc::Done(0) }
            "NtWaitForAlertByThreadId" => { // (address, timeout*)
                let cur = self.cur;
                if self.threads[cur].alerted { self.threads[cur].alerted = false; self.cur_deadline = None; Tc::Done(0) } else { self.wait_or_block(a[1])? }
            }
            "NtAlertThreadByThreadId" | "NtAlertThread" => {
                let tid = a[0];
                if let Some(t) = self.threads.iter_mut().find(|t| t.tid == tid) { t.alerted = true; }
                Tc::Done(0)
            }
            "NtCreateThreadEx" => {
                // (PHANDLE, access, oa, process, start, arg, flags, zero_bits, stack_size, max_stack, attrlist)
                let idx = self.new_thread(a[4], a[5], a[6] & 1 != 0, a[8])?;
                let h = self.new_handle(Obj::Thread(idx));
                self.w32(a[0], h)?;
                let (pid, tid) = (0x20u32, self.threads[idx].tid);
                if a[10] != 0 { // PS_ATTRIBUTE_LIST: look for the client-id output
                    let total = self.r32(a[10])?;
                    let mut o = 4u32;
                    while o + 16 <= total {
                        let attr = self.r32(a[10] + o)?;
                        if attr & 0xFFFF == 3 { let p = self.r32(a[10] + o + 8)?; if p != 0 { self.w32(p, pid)?; self.w32(p + 4, tid)?; } }
                        o += 16;
                    }
                }
                Tc::Done(0)
            }
            "NtResumeThread" | "NtSuspendThread" => {
                let Some(i) = self.thread_of(a[0]) else { return Ok(Tc::Done(STATUS_INVALID_HANDLE)) };
                let prev = self.threads[i].suspend;
                if name == "NtResumeThread" { self.threads[i].suspend = prev.saturating_sub(1); } else { self.threads[i].suspend += 1; }
                if a[1] != 0 { self.w32(a[1], prev)?; }
                Tc::Done(0)
            }
            "NtTerminateThread" => {
                let Some(i) = self.thread_of(a[0]) else { return Ok(Tc::Done(STATUS_INVALID_HANDLE)) };
                self.threads[i].done = true;
                self.threads[i].exit_code = a[1];
                if i == self.cur {
                    if !self.switch_next() { self.exit = Some(a[1]); } // last thread: the process ends
                    Tc::Retry
                } else { Tc::Done(0) }
            }
            "NtQueryInformationThread" if a[1] == 0 => {
                let Some(i) = self.thread_of(a[0]) else { return Ok(Tc::Done(STATUS_INVALID_HANDLE)) };
                if a[3] < 28 { return Ok(Tc::Done(0xC000_0004)); }
                let t = &self.threads[i];
                let (st, teb, tid) = (if t.done { t.exit_code } else { 0x103 }, t.teb, t.tid);
                for (k, v) in [st, teb, 0x20, tid, 1, 8, 0].iter().enumerate() { self.w32(a[2] + k as u32 * 4, *v)?; }
                if a[4] != 0 { self.w32(a[4], 28)?; }
                Tc::Done(0)
            }
            "NtOpenThread" => { // (PHANDLE, access, oa, client_id*)
                let tid = self.r32(a[3] + 4)?;
                match self.threads.iter().position(|t| t.tid == tid) { Some(i) => { let h = self.new_handle(Obj::Thread(i)); self.w32(a[0], h)?; Tc::Done(0) } None => Tc::Done(0xC000_000B) }
            }
            _ => Tc::NotMine,
        })
    }
}
