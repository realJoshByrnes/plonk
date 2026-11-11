//! Guest memory: guest_addr -> base + guest_addr. In wasm, `base` points into linear memory
//! so host/Wine code can dereference guest pointers directly (no copying).

#[derive(Debug, PartialEq, Clone, Copy)]
pub struct Fault(pub u32);

pub struct Mem {
    base: *mut u8,
    len: u32,
    /// Accesses below this address fault (the NULL page).
    guard: u32,
    _own: Option<Vec<u8>>,
    /// One flag byte per 4KB page: set when the CPU has cached decoded code from that page.
    cf: Vec<u8>,
    /// Set when a guest write hit a flagged page (the CPU then drops its decoded-code cache).
    pub smc: bool,
    /// Sparse memories only: one byte per 4KB page (bit 0 = accessible, bit 7 = backed by committed host memory).
    perm: Vec<u8>,
    sparse: bool,
}

const P_OK: u8 = 1;
const P_COMMIT: u8 = 0x80;

#[cfg(windows)]
extern "system" {
    fn VirtualAlloc(addr: *mut u8, size: usize, ty: u32, prot: u32) -> *mut u8;
    fn VirtualFree(addr: *mut u8, size: usize, ty: u32) -> i32;
}

impl Drop for Mem {
    fn drop(&mut self) {
        #[cfg(windows)]
        if self.sparse && self._own.is_none() { unsafe { VirtualFree(self.base, 0, 0x8000); } }
    }
}

impl Mem {
    /// Owned zeroed memory (tests / native).
    pub fn new_owned(len: u32) -> Self {
        let mut v = vec![0u8; len as usize];
        Mem { base: v.as_mut_ptr(), len, guard: 0, _own: Some(v), cf: vec![0; (len >> 12) as usize + 2], smc: false, perm: Vec::new(), sparse: false }
    }
    /// # Safety
    /// `base..base+len` must be valid, writable, and outlive the Mem.
    pub unsafe fn from_raw(base: *mut u8, len: u32) -> Self {
        Mem { base, len, guard: 0, _own: None, cf: vec![0; (len >> 12) as usize + 2], smc: false, perm: Vec::new(), sparse: false }
    }
    /// A sparse address space of `len` bytes (a multiple of 4KB): nothing is accessible until `map` is called, and host
    /// memory is only committed for mapped pages (on Windows; elsewhere the whole range is allocated lazily by the OS).
    pub fn new_sparse(len: u32) -> Self {
        let pages = (len >> 12) as usize;
        #[cfg(windows)]
        {
            let base = unsafe { VirtualAlloc(core::ptr::null_mut(), len as usize, 0x2000, 1) }; // MEM_RESERVE, PAGE_NOACCESS
            assert!(!base.is_null(), "cannot reserve guest address space");
            Mem { base, len, guard: 0, _own: None, cf: vec![0; pages + 2], smc: false, perm: vec![0; pages + 2], sparse: true }
        }
        #[cfg(not(windows))]
        {
            let mut m = Self::new_owned(len);
            m.perm = vec![P_COMMIT; pages + 2];
            m.sparse = true;
            m
        }
    }
    pub fn is_sparse(&self) -> bool { self.sparse }
    fn code_in(&self, a: u32, n: u32) -> bool { (a >> 12..=(a + n - 1) >> 12).any(|p| self.cf[p as usize] != 0) }
    /// Sparse: (re)map [a, a+n) as fresh zero pages, accessible or not. Addresses must be page aligned and in range.
    pub fn map(&mut self, a: u32, n: u32, accessible: bool) {
        if n == 0 || !self.sparse { return; }
        if self.code_in(a, n) { self.smc = true; }
        let (p0, p1) = ((a >> 12) as usize, ((a + n - 1) >> 12) as usize);
        let was = self.perm[p0..=p1].iter().any(|&f| f & P_COMMIT != 0);
        #[cfg(windows)]
        unsafe {
            if was { VirtualFree(self.base.add(a as usize), n as usize, 0x4000); } // MEM_DECOMMIT: drops the old contents
            if accessible { assert!(!VirtualAlloc(self.base.add(a as usize), n as usize, 0x1000, 4).is_null(), "out of host memory"); }
            self.perm[p0..=p1].fill(if accessible { P_OK | P_COMMIT } else { 0 });
        }
        #[cfg(not(windows))]
        {
            if was { unsafe { core::ptr::write_bytes(self.base.add(a as usize), 0, n as usize) }; }
            self.perm[p0..=p1].fill(if accessible { P_OK | P_COMMIT } else { P_COMMIT });
        }
    }
    /// Sparse: change accessibility of [a, a+n), keeping the contents.
    pub fn protect(&mut self, a: u32, n: u32, accessible: bool) {
        if n == 0 || !self.sparse || a.checked_add(n).map_or(true, |e| e > self.len) { return; }
        for p in (a >> 12) as usize..=((a + n - 1) >> 12) as usize {
            let f = self.perm[p];
            if accessible && f & P_COMMIT == 0 {
                #[cfg(windows)]
                unsafe { assert!(!VirtualAlloc(self.base.add(p << 12), 4096, 0x1000, 4).is_null(), "out of host memory"); }
                self.perm[p] = P_OK | P_COMMIT;
            } else { self.perm[p] = (f & P_COMMIT) | if accessible { P_OK } else { 0 }; }
        }
    }
    /// Sparse: copy pages [a, a+n) (contents and accessibility) from another sparse memory (fork).
    pub fn copy_pages_from(&mut self, o: &Mem, a: u32, n: u32) {
        if n == 0 { return; }
        for p in (a >> 12) as usize..=((a + n - 1) >> 12) as usize {
            let f = o.perm[p];
            if f & P_COMMIT == 0 { continue; }
            self.protect((p << 12) as u32, 4096, true);
            unsafe { core::ptr::copy_nonoverlapping(o.base.add(p << 12), self.base.add(p << 12), 4096) };
            self.perm[p] = f;
        }
    }
    /// Make accesses to [0, n) fault.
    pub fn set_guard(&mut self, n: u32) { self.guard = n; }
    pub fn len(&self) -> u32 { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    pub fn base(&self) -> *mut u8 { self.base }

    fn chk(&self, a: u32, n: u32) -> Result<*mut u8, Fault> {
        match a.checked_add(n) {
            Some(e) if e <= self.len && a >= self.guard => {
                if self.sparse && !self.pages_ok(a, n) { return Err(Fault(a)); }
                Ok(unsafe { self.base.add(a as usize) })
            }
            _ => Err(Fault(a)),
        }
    }
    #[inline(always)]
    fn pages_ok(&self, a: u32, n: u32) -> bool {
        if n <= 4096 {
            // in range, so both page indexes are valid
            unsafe { *self.perm.get_unchecked((a >> 12) as usize) & *self.perm.get_unchecked(((a + n.max(1) - 1) >> 12) as usize) & P_OK != 0 }
        } else { self.perm[(a >> 12) as usize..=((a + n - 1) >> 12) as usize].iter().all(|&f| f & P_OK != 0) }
    }
    pub fn r8(&self, a: u32) -> Result<u8, Fault> { Ok(unsafe { *self.chk(a, 1)? }) }
    pub fn r16(&self, a: u32) -> Result<u16, Fault> { Ok(unsafe { (self.chk(a, 2)? as *const u16).read_unaligned() }) }
    pub fn r32(&self, a: u32) -> Result<u32, Fault> { Ok(unsafe { (self.chk(a, 4)? as *const u32).read_unaligned() }) }
    /// Bounds check for a write, plus self-modifying-code detection on flagged pages.
    #[inline(always)]
    fn chkw(&mut self, a: u32, n: u32) -> Result<*mut u8, Fault> {
        let p = self.chk(a, n)?;
        // chk succeeded, so a + n <= len and both page indexes are in range of cf (len/4096 + 2 entries)
        let hit = unsafe { *self.cf.get_unchecked((a >> 12) as usize) | *self.cf.get_unchecked(((a + n.max(1) - 1) >> 12) as usize) };
        if hit != 0 { self.smc = true; }
        Ok(p)
    }
    /// Flag [a, a+n) as holding cached decoded code.
    pub fn mark_code(&mut self, a: u32, n: u32) {
        if n == 0 || a.checked_add(n).map_or(true, |e| e > self.len) { return; }
        for p in (a >> 12)..=((a + n - 1) >> 12) { self.cf[p as usize] = 1; }
    }
    /// Forget all code flags (the CPU dropped its cache).
    pub fn clear_code(&mut self) { self.cf.fill(0); self.smc = false; }
    pub fn w8(&mut self, a: u32, v: u8) -> Result<(), Fault> { unsafe { *self.chkw(a, 1)? = v }; Ok(()) }
    pub fn w16(&mut self, a: u32, v: u16) -> Result<(), Fault> { unsafe { (self.chkw(a, 2)? as *mut u16).write_unaligned(v) }; Ok(()) }
    pub fn w32(&mut self, a: u32, v: u32) -> Result<(), Fault> { unsafe { (self.chkw(a, 4)? as *mut u32).write_unaligned(v) }; Ok(()) }
    pub fn write_bytes(&mut self, a: u32, b: &[u8]) -> Result<(), Fault> {
        let p = self.chkw(a, b.len() as u32)?;
        unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len()) };
        Ok(())
    }
    pub fn read_bytes(&self, a: u32, n: u32) -> Result<&[u8], Fault> {
        let p = self.chk(a, n)?;
        Ok(unsafe { core::slice::from_raw_parts(p, n as usize) })
    }
}
