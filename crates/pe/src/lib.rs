//! PE32 (i386) loader: maps sections, applies relocations, resolves imports via callback.
use plonk_mem::Mem;

pub struct Image {
    pub base: u32,
    pub entry: u32,
    pub size: u32,
}

fn u16_at(f: &[u8], o: usize) -> Result<u32, String> {
    f.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as u32).ok_or("truncated PE".into())
}
fn u32_at(f: &[u8], o: usize) -> Result<u32, String> {
    f.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or("truncated PE".into())
}
fn cstr(m: &Mem, a: u32) -> Result<String, String> {
    let mut v = Vec::new();
    for i in 0..512 {
        let c = m.r8(a + i).map_err(|_| "bad string rva")?;
        if c == 0 { break; }
        v.push(c);
    }
    Ok(String::from_utf8_lossy(&v).into_owned())
}

/// Load `file` at `base` (None = preferred ImageBase) into fresh (zeroed) memory.
/// `resolve(dll, name)` returns the address to store in the IAT slot (ordinals are "#<n>").
pub fn load(file: &[u8], mem: &mut Mem, base: Option<u32>, resolve: &mut dyn FnMut(&str, &str) -> u32) -> Result<Image, String> {
    if file.get(0..2) != Some(b"MZ") { return Err("no MZ".into()); }
    let pe = u32_at(file, 0x3C)? as usize;
    if file.get(pe..pe + 4) != Some(b"PE\0\0") { return Err("no PE sig".into()); }
    if u16_at(file, pe + 4)? != 0x14c { return Err("not i386".into()); }
    let nsec = u16_at(file, pe + 6)? as usize;
    let optsz = u16_at(file, pe + 20)? as usize;
    let opt = pe + 24;
    if u16_at(file, opt)? != 0x10b { return Err("not PE32".into()); }
    let entry_rva = u32_at(file, opt + 16)?;
    let pref = u32_at(file, opt + 28)?;
    let size = u32_at(file, opt + 56)?;
    let hdrs = u32_at(file, opt + 60)? as usize;
    let base = base.unwrap_or(pref);
    let e = |_| "image does not fit in guest memory".to_string();
    mem.write_bytes(base, file.get(..hdrs).ok_or("bad headers")?).map_err(e)?;
    for i in 0..nsec {
        let s = opt + optsz + i * 40;
        let (vsz, va, rsz, rptr) = (u32_at(file, s + 8)?, u32_at(file, s + 12)?, u32_at(file, s + 16)?, u32_at(file, s + 20)? as usize);
        let n = rsz.min(vsz) as usize;
        let data = file.get(rptr..rptr + n).ok_or("bad section raw data")?;
        mem.write_bytes(base + va, data).map_err(e)?;
    }
    // relocations (data dir 5)
    let delta = base.wrapping_sub(pref);
    let (rrva, rsz) = (u32_at(file, opt + 96 + 5 * 8)?, u32_at(file, opt + 96 + 5 * 8 + 4)?);
    if delta != 0 && rrva != 0 {
        let (mut p, end) = (base + rrva, base + rrva + rsz);
        while p < end {
            let (page, blk) = (mem.r32(p).map_err(e)?, mem.r32(p + 4).map_err(e)?);
            if blk < 8 { break; }
            for k in 0..(blk - 8) / 2 {
                let ent = mem.r16(p + 8 + k * 2).map_err(e)? as u32;
                match ent >> 12 {
                    0 => {}
                    3 => {
                        let a = base + page + (ent & 0xFFF);
                        let v = mem.r32(a).map_err(e)?;
                        mem.w32(a, v.wrapping_add(delta)).map_err(e)?;
                    }
                    t => return Err(format!("unsupported reloc type {t}")),
                }
            }
            p += blk;
        }
    } else if delta != 0 {
        return Err("image needs relocation but has none".into());
    }
    // imports (data dir 1)
    let irva = u32_at(file, opt + 96 + 8)?;
    if irva != 0 {
        let mut d = base + irva;
        loop {
            let (oft, name, ft) = (mem.r32(d).map_err(e)?, mem.r32(d + 12).map_err(e)?, mem.r32(d + 16).map_err(e)?);
            if name == 0 { break; }
            let dll = cstr(mem, base + name)?;
            let mut t = base + if oft != 0 { oft } else { ft };
            let mut slot = base + ft;
            loop {
                let v = mem.r32(t).map_err(e)?;
                if v == 0 { break; }
                let fname = if v & 0x8000_0000 != 0 { format!("#{}", v & 0xFFFF) } else { cstr(mem, base + (v & 0x7FFF_FFFF) + 2)? };
                let addr = resolve(&dll, &fname);
                mem.w32(slot, addr).map_err(e)?;
                t += 4;
                slot += 4;
            }
            d += 20;
        }
    }
    Ok(Image { base, entry: base + entry_rva, size })
}
