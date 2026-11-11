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
}

impl Mem {
    /// Owned zeroed memory (tests / native).
    pub fn new_owned(len: u32) -> Self {
        let mut v = vec![0u8; len as usize];
        Mem { base: v.as_mut_ptr(), len, guard: 0, _own: Some(v), cf: vec![0; (len >> 12) as usize + 2], smc: false }
    }
    /// # Safety
    /// `base..base+len` must be valid, writable, and outlive the Mem.
    pub unsafe fn from_raw(base: *mut u8, len: u32) -> Self {
        Mem { base, len, guard: 0, _own: None, cf: vec![0; (len >> 12) as usize + 2], smc: false }
    }
    /// Make accesses to [0, n) fault.
    pub fn set_guard(&mut self, n: u32) { self.guard = n; }
    pub fn len(&self) -> u32 { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    pub fn base(&self) -> *mut u8 { self.base }

    fn chk(&self, a: u32, n: u32) -> Result<*mut u8, Fault> {
        match a.checked_add(n) {
            Some(e) if e <= self.len && a >= self.guard => Ok(unsafe { self.base.add(a as usize) }),
            _ => Err(Fault(a)),
        }
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
