//! Guest memory. Two layouts:
//! - flat: guest_addr -> base + guest_addr. In wasm, `base` points into linear memory so host code can dereference guest
//!   pointers directly (no copying). Used by the hand-written Windows layer.
//! - paged ("sparse"): a page table of host pointers, one 4KB page at a time, so several large, mostly empty address
//!   spaces (Linux processes) fit in a 32-bit host such as wasm32. Pages can be mapped, unmapped and made inaccessible.

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
    /// Paged memories only: per page, the host address of its 4KB block (page aligned) with bit 0 set when accessible; 0 = no page.
    tbl: Vec<usize>,
    sparse: bool,
    chunks: Vec<*mut u8>,
    free: Vec<*mut u8>,
    fresh: Vec<*mut u8>,
    /// Paged: backing for `read_bytes` results that span pages (valid until the next such read).
    scratch: core::cell::UnsafeCell<Vec<u8>>,
}

const P_OK: usize = 1;
/// The page belongs to someone else (a shared file mapping): never recycled by this memory.
const P_EXT: usize = 2;
/// An external page that must not be written: the first write gives this memory its own copy (always with P_EXT).
const P_COW: usize = 4;
const CHUNK_PAGES: usize = 256;

fn chunk_layout() -> std::alloc::Layout { std::alloc::Layout::from_size_align(CHUNK_PAGES * 4096, 4096).unwrap() }

impl Drop for Mem {
    fn drop(&mut self) {
        for &c in &self.chunks { unsafe { std::alloc::dealloc(c, chunk_layout()) }; }
    }
}

impl Mem {
    fn with(base: *mut u8, len: u32, own: Option<Vec<u8>>) -> Self {
        Mem { base, len, guard: 0, _own: own, cf: vec![0; (len >> 12) as usize + 2], smc: false, tbl: Vec::new(), sparse: false,
            chunks: Vec::new(), free: Vec::new(), fresh: Vec::new(), scratch: core::cell::UnsafeCell::new(Vec::new()) }
    }
    /// Owned zeroed flat memory (tests / native).
    pub fn new_owned(len: u32) -> Self {
        let mut v = vec![0u8; len as usize];
        Self::with(v.as_mut_ptr(), len, Some(v))
    }
    /// # Safety
    /// `base..base+len` must be valid, writable, and outlive the Mem.
    pub unsafe fn from_raw(base: *mut u8, len: u32) -> Self { Self::with(base, len, None) }

    /// A paged address space of `len` bytes (a multiple of 4KB): nothing is accessible until `map` is called, and host
    /// memory is only used for mapped pages.
    pub fn new_sparse(len: u32) -> Self {
        let mut m = Self::with(core::ptr::null_mut(), len, None);
        m.tbl = vec![0; (len >> 12) as usize + 2];
        m.sparse = true;
        m
    }
    pub fn is_sparse(&self) -> bool { self.sparse }
    /// Host bytes held by a paged memory.
    pub fn committed(&self) -> usize { self.chunks.len() * CHUNK_PAGES * 4096 }

    /// A zeroed 4KB page: from the never-used list, a recycled one (cleared here), or a new chunk.
    fn take_page(&mut self) -> *mut u8 {
        if let Some(p) = self.fresh.pop() { return p; }
        if let Some(p) = self.free.pop() {
            unsafe { core::ptr::write_bytes(p, 0, 4096) };
            return p;
        }
        let c = unsafe { std::alloc::alloc_zeroed(chunk_layout()) };
        assert!(!c.is_null(), "out of host memory");
        self.chunks.push(c);
        for i in (1..CHUNK_PAGES).rev() { self.fresh.push(unsafe { c.add(i * 4096) }); }
        c
    }

    fn code_in(&self, a: u32, n: u32) -> bool { (a >> 12..=(a + n - 1) >> 12).any(|p| self.cf[p as usize] != 0) }
    /// Paged: (re)map [a, a+n) as fresh zero pages, accessible or not. Addresses must be page aligned and in range.
    pub fn map(&mut self, a: u32, n: u32, accessible: bool) {
        if n == 0 || !self.sparse { return; }
        if self.code_in(a, n) { self.smc = true; }
        for p in (a >> 12) as usize..=((a + n - 1) >> 12) as usize {
            self.drop_page(p);
            self.tbl[p] = if accessible { self.take_page() as usize | P_OK } else { 0 };
        }
    }
    /// Paged: change accessibility of [a, a+n), keeping the contents.
    pub fn protect(&mut self, a: u32, n: u32, accessible: bool) {
        if n == 0 || !self.sparse || a.checked_add(n).map_or(true, |e| e > self.len) { return; }
        for p in (a >> 12) as usize..=((a + n - 1) >> 12) as usize {
            let e = self.tbl[p];
            self.tbl[p] = if !accessible { e & !P_OK } else if e == 0 { self.take_page() as usize | P_OK } else { e | P_OK };
        }
    }
    /// Paged: copy pages [a, a+n) (contents and accessibility) from another paged memory (fork).
    pub fn copy_pages_from(&mut self, o: &Mem, a: u32, n: u32) {
        if n == 0 { return; }
        for p in (a >> 12) as usize..=((a + n - 1) >> 12) as usize {
            let e = o.tbl[p];
            if e & !0xFFF == 0 { continue; }
            if e & P_EXT != 0 { self.drop_page(p); self.tbl[p] = e; continue; } // shared mappings stay shared
            let mine = if self.tbl[p] & !0xFFF != 0 && self.tbl[p] & P_EXT == 0 { (self.tbl[p] & !0xFFF) as *mut u8 } else { self.take_page() };
            unsafe { core::ptr::copy_nonoverlapping((e & !0xFFF) as *const u8, mine, 4096) };
            self.tbl[p] = mine as usize | (e & P_OK);
        }
    }
    /// Paged: visit every page that has host memory: (page index, accessible, the external page's address if it is a shared
    /// one, its 4096 bytes).
    pub fn each_page(&self, mut f: impl FnMut(u32, bool, Option<usize>, &[u8])) {
        for (i, &e) in self.tbl.iter().enumerate() {
            let ptr = e & !0xFFF;
            if ptr != 0 { f(i as u32, e & P_OK != 0, (e & P_EXT != 0).then_some(ptr), unsafe { core::slice::from_raw_parts(ptr as *const u8, 4096) }); }
        }
    }
    /// Give the page back to the free list (unless it is an external one) and clear its entry.
    fn drop_page(&mut self, p: usize) {
        let e = self.tbl[p];
        if e & !0xFFF != 0 && e & P_EXT == 0 { self.free.push((e & !0xFFF) as *mut u8); }
        self.tbl[p] = 0;
    }
    /// Paged: make the page at `a` an alias of an external 4KB block (4KB aligned, must outlive this memory): shared mappings.
    pub fn map_ext(&mut self, a: u32, ptr: *mut u8, accessible: bool) {
        let p = (a >> 12) as usize;
        if !self.sparse || p >= self.tbl.len() || ptr as usize & 0xFFF != 0 { return; }
        if self.cf[p] != 0 { self.smc = true; }
        self.drop_page(p);
        self.tbl[p] = ptr as usize | P_EXT | if accessible { P_OK } else { 0 };
    }
    /// Paged: make the page at `a` a copy-on-write view of an external 4KB block (4KB aligned, must outlive this memory and
    /// never change): reads use it directly, the first write replaces it with a private copy.
    pub fn map_cow(&mut self, a: u32, ptr: *const u8, accessible: bool) {
        let p = (a >> 12) as usize;
        if !self.sparse || p >= self.tbl.len() || ptr as usize & 0xFFF != 0 { return; }
        if self.cf[p] != 0 { self.smc = true; }
        self.drop_page(p);
        self.tbl[p] = ptr as usize | P_EXT | P_COW | if accessible { P_OK } else { 0 };
    }
    /// The write that found a copy-on-write page: give the page its own copy and return the host address for `a` in it.
    #[cold]
    fn cow_break(&mut self, a: u32) -> *mut u8 {
        let i = (a >> 12) as usize;
        let e = self.tbl[i];
        let mine = self.take_page();
        unsafe { core::ptr::copy_nonoverlapping((e & !0xFFF) as *const u8, mine, 4096) };
        self.tbl[i] = mine as usize | (e & P_OK);
        unsafe { mine.add((a & 0xFFF) as usize) }
    }
    /// Host bytes of the pages this memory owns itself (not the shared or copy-on-write ones it only refers to).
    pub fn own_bytes(&self) -> usize { self.tbl.iter().filter(|&&e| e & !0xFFF != 0 && e & P_EXT == 0).count() * 4096 }
    /// Paged: give page `idx` these contents and accessibility (snapshot restore).
    pub fn set_page(&mut self, idx: u32, accessible: bool, data: &[u8]) {
        let i = idx as usize;
        if i >= self.tbl.len() || data.len() != 4096 { return; }
        let p = if self.tbl[i] & !0xFFF != 0 { (self.tbl[i] & !0xFFF) as *mut u8 } else { self.take_page() };
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), p, 4096) };
        self.tbl[i] = p as usize | if accessible { P_OK } else { 0 };
    }
    /// Make accesses to [0, n) fault (flat memories).
    pub fn set_guard(&mut self, n: u32) { self.guard = n; }
    pub fn len(&self) -> u32 { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    pub fn base(&self) -> *mut u8 { self.base }

    /// Host pointer for an access of `n` bytes at `a`. Paged memories only answer accesses that stay within one page here;
    /// the callers handle page-crossing ones.
    #[inline(always)]
    fn chk(&self, a: u32, n: u32) -> Result<*mut u8, Fault> {
        if self.sparse {
            let off = (a & 0xFFF) as usize;
            if off + n as usize > 4096 { return Err(Fault(a)); }
            return match self.tbl.get((a >> 12) as usize) {
                Some(&e) if e & P_OK != 0 => Ok((e & !0xFFF) as *mut u8).map(|p| unsafe { p.add(off) }),
                _ => Err(Fault(a)),
            };
        }
        match a.checked_add(n) {
            Some(e) if e <= self.len && a >= self.guard => Ok(unsafe { self.base.add(a as usize) }),
            _ => Err(Fault(a)),
        }
    }
    #[inline(always)]
    fn crosses(&self, a: u32, n: u32) -> bool { self.sparse && (a & 0xFFF) + n > 4096 }
    #[cold]
    fn read_slow(&self, a: u32, out: &mut [u8]) -> Result<(), Fault> {
        for (i, b) in out.iter_mut().enumerate() { *b = unsafe { *self.chk(a.checked_add(i as u32).ok_or(Fault(a))?, 1)? }; }
        Ok(())
    }
    #[cold]
    fn write_slow(&mut self, a: u32, b: &[u8]) -> Result<(), Fault> {
        // check every page first so a faulting write changes nothing
        let end = a.checked_add(b.len() as u32).ok_or(Fault(a))?;
        let mut p = a;
        while p < end { self.chk(p, 1)?; p = (p | 0xFFF).wrapping_add(1); if p == 0 { break; } }
        let mut done = 0usize;
        while done < b.len() {
            let at = a + done as u32;
            let k = (4096 - (at & 0xFFF) as usize).min(b.len() - done);
            let dst = self.chkw(at, k as u32)?;
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr().add(done), dst, k) };
            done += k;
        }
        Ok(())
    }
    #[inline(always)]
    pub fn r8(&self, a: u32) -> Result<u8, Fault> { Ok(unsafe { *self.chk(a, 1)? }) }
    #[inline(always)]
    pub fn r16(&self, a: u32) -> Result<u16, Fault> {
        if self.crosses(a, 2) { let mut b = [0u8; 2]; self.read_slow(a, &mut b)?; return Ok(u16::from_le_bytes(b)); }
        Ok(unsafe { (self.chk(a, 2)? as *const u16).read_unaligned() })
    }
    #[inline(always)]
    pub fn r32(&self, a: u32) -> Result<u32, Fault> {
        if self.crosses(a, 4) { let mut b = [0u8; 4]; self.read_slow(a, &mut b)?; return Ok(u32::from_le_bytes(b)); }
        Ok(unsafe { (self.chk(a, 4)? as *const u32).read_unaligned() })
    }
    /// Bounds check for a write, plus self-modifying-code detection on flagged pages.
    #[inline(always)]
    fn chkw(&mut self, a: u32, n: u32) -> Result<*mut u8, Fault> {
        let mut p = self.chk(a, n)?;
        // (chk succeeded on a paged memory: the access is within one page whose entry exists)
        if self.sparse && unsafe { *self.tbl.get_unchecked((a >> 12) as usize) } & P_COW != 0 { p = self.cow_break(a); }
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
    #[inline(always)]
    pub fn w8(&mut self, a: u32, v: u8) -> Result<(), Fault> { unsafe { *self.chkw(a, 1)? = v }; Ok(()) }
    #[inline(always)]
    pub fn w16(&mut self, a: u32, v: u16) -> Result<(), Fault> {
        if self.crosses(a, 2) { return self.write_slow(a, &v.to_le_bytes()); }
        unsafe { (self.chkw(a, 2)? as *mut u16).write_unaligned(v) };
        Ok(())
    }
    #[inline(always)]
    pub fn w32(&mut self, a: u32, v: u32) -> Result<(), Fault> {
        if self.crosses(a, 4) { return self.write_slow(a, &v.to_le_bytes()); }
        unsafe { (self.chkw(a, 4)? as *mut u32).write_unaligned(v) };
        Ok(())
    }
    pub fn write_bytes(&mut self, a: u32, b: &[u8]) -> Result<(), Fault> {
        if b.is_empty() { return Ok(()); }
        if self.crosses(a, b.len() as u32) || (self.sparse && b.len() > 4096) { return self.write_slow(a, b); }
        let p = self.chkw(a, b.len() as u32)?;
        unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len()) };
        Ok(())
    }
    /// Borrow guest bytes. On a paged memory a range that spans pages is copied into an internal buffer, so the result is
    /// only valid until the next page-spanning `read_bytes` on this memory: use it right away.
    pub fn read_bytes(&self, a: u32, n: u32) -> Result<&[u8], Fault> {
        if n == 0 { return Ok(&[]); }
        if self.sparse && ((a & 0xFFF) as u64 + n as u64 > 4096) {
            let buf = unsafe { &mut *self.scratch.get() };
            buf.clear();
            let end = a.checked_add(n).ok_or(Fault(a))?;
            let mut at = a;
            while at < end {
                let k = (4096 - (at & 0xFFF)).min(end - at);
                let p = self.chk(at, k)?;
                buf.extend_from_slice(unsafe { core::slice::from_raw_parts(p, k as usize) });
                at += k;
            }
            return Ok(&buf[..]);
        }
        let p = self.chk(a, n)?;
        Ok(unsafe { core::slice::from_raw_parts(p, n as usize) })
    }
}
