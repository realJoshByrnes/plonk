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
}

impl Mem {
    /// Owned zeroed memory (tests / native).
    pub fn new_owned(len: u32) -> Self {
        let mut v = vec![0u8; len as usize];
        Mem { base: v.as_mut_ptr(), len, guard: 0, _own: Some(v) }
    }
    /// # Safety
    /// `base..base+len` must be valid, writable, and outlive the Mem.
    pub unsafe fn from_raw(base: *mut u8, len: u32) -> Self {
        Mem { base, len, guard: 0, _own: None }
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
    pub fn w8(&mut self, a: u32, v: u8) -> Result<(), Fault> { unsafe { *self.chk(a, 1)? = v }; Ok(()) }
    pub fn w16(&mut self, a: u32, v: u16) -> Result<(), Fault> { unsafe { (self.chk(a, 2)? as *mut u16).write_unaligned(v) }; Ok(()) }
    pub fn w32(&mut self, a: u32, v: u32) -> Result<(), Fault> { unsafe { (self.chk(a, 4)? as *mut u32).write_unaligned(v) }; Ok(()) }
    pub fn write_bytes(&mut self, a: u32, b: &[u8]) -> Result<(), Fault> {
        let p = self.chk(a, b.len() as u32)?;
        unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len()) };
        Ok(())
    }
    pub fn read_bytes(&self, a: u32, n: u32) -> Result<&[u8], Fault> {
        let p = self.chk(a, n)?;
        Ok(unsafe { core::slice::from_raw_parts(p, n as usize) })
    }
}
