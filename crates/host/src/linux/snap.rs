//! Snapshots of the whole kernel (all processes, their memory, descriptors, sockets, the writable file tree, the display
//! state), so a booted Wine session (wineserver + services + explorer, idle) can be restored instead of booted again.
//! The read-only file layer is not included: load the same tar archives before restoring. Files that were only read are
//! stored by name; identical memory pages are stored once.
use super::*;

const MAGIC: &[u8; 8] = b"PLNKSNP4";

#[derive(Default)]
struct W(Vec<u8>);
impl W {
    fn u8(&mut self, v: u8) { self.0.push(v); }
    fn u32(&mut self, v: u32) { self.0.extend_from_slice(&v.to_le_bytes()); }
    fn u64(&mut self, v: u64) { self.0.extend_from_slice(&v.to_le_bytes()); }
    fn bytes(&mut self, b: &[u8]) { self.u32(b.len() as u32); self.0.extend_from_slice(b); }
    fn str(&mut self, s: &str) { self.bytes(s.as_bytes()); }
    fn opt(&mut self, v: Option<u64>) { match v { Some(x) => { self.u8(1); self.u64(x); } None => self.u8(0) } }
}

struct R<'a> { d: &'a [u8], p: usize }
impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.d.get(self.p..self.p + n).ok_or("snapshot truncated")?;
        self.p += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> { Ok(self.take(1)?[0]) }
    fn u32(&mut self) -> Result<u32, String> { Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn u64(&mut self) -> Result<u64, String> { Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap())) }
    fn bytes(&mut self) -> Result<&'a [u8], String> { let n = self.u32()? as usize; self.take(n) }
    fn str(&mut self) -> Result<String, String> { Ok(String::from_utf8_lossy(self.bytes()?).into_owned()) }
    fn opt(&mut self) -> Result<Option<u64>, String> { Ok(if self.u8()? != 0 { Some(self.u64()?) } else { None }) }
}

fn put_cpu(w: &mut W, c: &Cpu) {
    w.u64(c.retired);
    for v in c.r { w.u32(v); }
    for v in [c.eip, c.eflags, c.fs_base, c.gs_base, c.tls[0], c.tls[1], c.tls[2], c.sel_fs, c.sel_gs, c.ftop, c.fsw, c.fcw, c.mxcsr] { w.u32(v); }
    w.u64(c.tsc);
    for v in c.fst { w.u64(v.to_bits()); }
    for v in c.xmm { w.0.extend_from_slice(&v.to_le_bytes()); }
}

fn get_cpu(r: &mut R) -> Result<Cpu, String> {
    let mut c = LinuxProc::new_cpu();
    c.retired = r.u64()?;
    for i in 0..8 { c.r[i] = r.u32()?; }
    c.eip = r.u32()?; c.eflags = r.u32()?; c.fs_base = r.u32()?; c.gs_base = r.u32()?;
    for i in 0..3 { c.tls[i] = r.u32()?; }
    c.sel_fs = r.u32()?; c.sel_gs = r.u32()?; c.ftop = r.u32()?; c.fsw = r.u32()?; c.fcw = r.u32()?; c.mxcsr = r.u32()?;
    c.tsc = r.u64()?;
    for i in 0..8 { c.fst[i] = f64::from_bits(r.u64()?); }
    for i in 0..8 { c.xmm[i] = u128::from_le_bytes(r.take(16)?.try_into().unwrap()); }
    Ok(c)
}

fn page_hash(b: &[u8]) -> u64 {
    // FNV-1a over 8-byte words: only used to bucket candidate pages, equality is checked on the bytes
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for c in b.chunks_exact(8) { h = (h ^ u64::from_le_bytes(c.try_into().unwrap())).wrapping_mul(0x0000_0100_0000_01b3); }
    h
}

impl LinuxProc {
    /// Move the running process and thread back into `procs` (nothing is "current" afterwards).
    fn park(&mut self) {
        if self.cur.0 == usize::MAX { return; }
        let c = self.cpu.take_cache();
        self.swap_thread();
        self.procs[self.cur.0].cache = c;
        self.swap_proc();
        self.cur = (usize::MAX, 0);
    }

    /// Serialize the whole kernel. Call between `run`s.
    pub fn snapshot(&mut self) -> Vec<u8> {
        self.park();
        let mut w = W::default();
        w.0.extend_from_slice(MAGIC);
        w.u64(self.layer_sig);
        for v in [self.clock_ns, self.frame] { w.u64(v); }
        for v in [self.next_pid, self.screen.0, self.screen.1, self.cursor as u32, self.umask] { w.u32(v); }
        w.opt(self.exit_code.map(|c| c as u64));

        // file contents, by identity: `datas[i]` is referenced from tree nodes and open files
        let mut data_ids: HashMap<*const RefCell<Vec<u8>>, u32> = HashMap::new();
        let mut datas: Vec<Data> = Vec::new();
        let mut data_id = |d: &Data, datas: &mut Vec<Data>| *data_ids.entry(Rc::as_ptr(d)).or_insert_with(|| { datas.push(d.clone()); datas.len() as u32 - 1 });
        // open file descriptions, by identity
        let mut ofd_ids: HashMap<*const RefCell<Ofd>, u32> = HashMap::new();
        let mut ofds: Vec<Fd> = Vec::new();
        let mut ofd_id = |o: &Fd, ofds: &mut Vec<Fd>| *ofd_ids.entry(Rc::as_ptr(o)).or_insert_with(|| { ofds.push(o.clone()); ofds.len() as u32 - 1 });
        for p in &self.procs { for o in p.fds.values() { ofd_id(o, &mut ofds); } }
        for s in &self.socks { for m in &s.rx { for o in &m.fds { ofd_id(o, &mut ofds); } } }

        // the writable tree; a file that still equals its read-only original is stored by name only
        let mut tree = W::default();
        tree.u32(self.vfs.len() as u32);
        for (path, n) in &self.vfs {
            tree.str(path);
            match n {
                Node::Dir => tree.u8(0),
                Node::Link(t) => { tree.u8(1); tree.str(t); }
                Node::Sock(s) => { tree.u8(2); tree.u32(*s as u32); }
                Node::File(d) => { tree.u8(3); tree.u32(data_id(d, &mut datas)); tree.u8(self.pristine.contains(path) as u8); }
            }
        }
        let mut of = W::default();
        of.u32(ofds.len() as u32);
        for o in &ofds {
            let o = o.borrow();
            of.u32(o.flags);
            of.str(&o.path);
            match &o.kind {
                Kind::File { data, pos } => { of.u8(0); of.u32(data_id(data, &mut datas)); of.u64(*pos); }
                Kind::Dir { pos } => { of.u8(1); of.u32(*pos as u32); }
                Kind::Null => of.u8(2),
                Kind::Tty => of.u8(3),
                Kind::Sock(s) => { of.u8(4); of.u32(*s as u32); }
                Kind::Events => of.u8(5),
                Kind::Dsp => of.u8(6),
                Kind::Mixer => of.u8(7),
            }
        }
        // shared-mapping page stores (their pages are referenced from the process page lists below)
        let mut sh = W::default();
        let mut ext: HashMap<usize, (u32, u32)> = HashMap::new();
        sh.u32(self.shared.len() as u32);
        for (si, s) in self.shared.iter().enumerate() {
            sh.u32(data_id(&s.data, &mut datas));
            sh.u32(s.pages.len() as u32);
            for (pi, &p) in s.pages.iter().enumerate() { ext.insert(p as usize, (si as u32, pi as u32)); sh.0.extend_from_slice(unsafe { core::slice::from_raw_parts(p, 4096) }); }
        }
        // contents of the files that are not stored by name
        let pristine_data: std::collections::HashSet<*const RefCell<Vec<u8>>> =
            self.vfs.iter().filter_map(|(p, n)| match n { Node::File(d) if self.pristine.contains(p) => Some(Rc::as_ptr(d)), _ => None }).collect();
        w.u32(datas.len() as u32);
        for d in &datas { if pristine_data.contains(&Rc::as_ptr(d)) { w.u8(0); } else { w.u8(1); w.bytes(&d.borrow()); } }
        w.0.extend_from_slice(&tree.0);
        w.0.extend_from_slice(&of.0);
        w.0.extend_from_slice(&sh.0);

        w.u32(self.socks.len() as u32);
        for s in &self.socks {
            w.u32(s.peer.map_or(u32::MAX, |p| p as u32));
            w.u8(s.listening as u8 | (s.closed as u8) << 1 | (s.shut_wr as u8) << 2);
            w.u32(s.backlog.len() as u32);
            for b in &s.backlog { w.u32(*b as u32); }
            w.u32(s.rx.len() as u32);
            for m in &s.rx {
                w.bytes(&m.data);
                w.u32(m.off as u32);
                w.u32(m.fds.len() as u32);
                for o in &m.fds { w.u32(ofd_ids[&Rc::as_ptr(o)]); }
            }
        }

        // memory pages, each distinct content once
        let mut pool: Vec<u8> = Vec::new();
        let mut seen: HashMap<u64, Vec<u32>> = HashMap::new();
        let mut procs = W::default();
        procs.u32(self.procs.len() as u32);
        for p in &self.procs {
            for v in [p.pid, p.ppid, p.brk, p.brk_start, p.umask] { procs.u32(v); }
            procs.opt(p.exit.map(|c| c as u64));
            procs.u8(p.reaped as u8);
            procs.str(&p.cwd); procs.str(&p.exe); procs.str(&p.cmd);
            procs.u32(p.maps.len() as u32);
            for (&s, &e) in &p.maps { procs.u32(s); procs.u32(e); }
            procs.u32(p.sigact.len() as u32);
            for (&s, a) in &p.sigact { procs.u32(s); for v in a { procs.u32(*v); } }
            procs.u32(p.fds.len() as u32);
            for (&fd, o) in &p.fds { procs.u32(fd); procs.u32(ofd_ids[&Rc::as_ptr(o)]); procs.u8(p.cloexec.contains(&fd) as u8); }
            procs.u32(p.threads.len() as u32);
            for t in &p.threads {
                put_cpu(&mut procs, &t.cpu);
                procs.u32(t.tid); procs.u32(t.clear_tid);
                procs.opt(t.poll_deadline); procs.opt(t.wake_at);
                match t.state { St::Run => procs.u8(0), St::Futex(a) => { procs.u8(1); procs.u32(a); } St::Vfork(c) => { procs.u8(2); procs.u32(c as u32); } St::Dead => procs.u8(3) }
            }
            let mut pages = W::default();
            let mut n = 0u32;
            if p.exit.is_none() {
                p.mem.each_page(|idx, ok, ext_ptr, bytes| {
                    if let Some((si, pi)) = ext_ptr.and_then(|a| ext.get(&a)) { // an alias of a shared page
                        pages.u32(idx); pages.u32(u32::MAX); pages.u32(*si); pages.u32(*pi << 1 | ok as u32);
                        n += 1;
                        return;
                    }
                    let bucket = seen.entry(page_hash(bytes)).or_default();
                    let id = match bucket.iter().find(|&&i| &pool[i as usize * 4096..i as usize * 4096 + 4096] == bytes) {
                        Some(&i) => i,
                        None => { let i = (pool.len() / 4096) as u32; pool.extend_from_slice(bytes); bucket.push(i); i }
                    };
                    pages.u32(idx); pages.u32(id << 1 | ok as u32);
                    n += 1;
                });
            }
            procs.u32(n);
            procs.0.extend_from_slice(&pages.0);
        }
        w.u32((pool.len() / 4096) as u32);
        w.0.extend_from_slice(&pool);
        w.0.extend_from_slice(&procs.0);

        w.u32(self.windows.len() as u32);
        for win in self.windows.values() {
            for v in [win.hwnd, win.owner, win.style, win.ex_style, win.visible as u32, win.sw, win.sh] { w.u32(v); }
            for v in win.rect.iter().chain(&win.client) { w.u32(*v as u32); }
            w.str(&win.title);
            w.bytes(&win.pixels);
        }
        w.u32(self.zorder.len() as u32);
        for h in &self.zorder { w.u32(*h); }
        w.0
    }

    /// Replace the kernel state with a snapshot. The read-only file layer (`files`, tar archives) must already be loaded
    /// exactly as it was when the snapshot was taken.
    pub fn restore(&mut self, snap: &[u8]) -> Result<(), String> {
        if snap.len() < 8 || &snap[..8] != MAGIC { return Err("not a Plonk snapshot (or an old format)".into()); }
        let mut r = R { d: snap, p: 8 };
        if r.u64()? != self.layer_sig { return Err("snapshot was taken with different Wine archives (make it again)".into()); }
        self.park();
        self.procs.clear();
        self.vfs.clear();
        self.pristine.clear();
        self.socks.clear();
        self.clock_ns = r.u64()?; self.frame = r.u64()?;
        self.next_pid = r.u32()?; self.screen = (r.u32()?, r.u32()?); self.cursor = r.u32()? as i32; self.umask = r.u32()?;
        self.exit_code = r.opt()?.map(|c| c as u32);

        let mut datas: Vec<Option<Data>> = Vec::new();
        for _ in 0..r.u32()? { datas.push(if r.u8()? != 0 { Some(Rc::new(RefCell::new(r.bytes()?.to_vec()))) } else { None }); }
        for _ in 0..r.u32()? {
            let path = r.str()?;
            let node = match r.u8()? {
                0 => Node::Dir,
                1 => Node::Link(r.str()?),
                2 => Node::Sock(r.u32()? as usize),
                _ => {
                    let (id, pristine) = (r.u32()? as usize, r.u8()? != 0);
                    let slot = datas.get(id).ok_or("bad data id")?.clone();
                    let d = match slot {
                        Some(d) => { self.files.remove(&path); self.aliases.remove(&path); d } // shared with an earlier path, or stored
                        None => {
                            if !pristine { return Err(format!("snapshot: no contents for {path}")); }
                            let d = self.promote(&path, false).ok_or(format!("snapshot needs {path} from the file archives"))?;
                            datas[id] = Some(d.clone());
                            d
                        }
                    };
                    if pristine { self.pristine.insert(path.clone()); }
                    Node::File(d)
                }
            };
            self.vfs.insert(path, node);
        }
        let mut ofds: Vec<Fd> = Vec::new();
        for _ in 0..r.u32()? {
            let (flags, path) = (r.u32()?, r.str()?);
            let kind = match r.u8()? {
                0 => { let id = r.u32()? as usize; Kind::File { data: datas.get(id).cloned().flatten().ok_or("open file without contents")?, pos: r.u64()? } }
                1 => Kind::Dir { pos: r.u32()? as usize },
                2 => Kind::Null,
                3 => Kind::Tty,
                4 => Kind::Sock(r.u32()? as usize),
                6 => Kind::Dsp,
                7 => Kind::Mixer,
                _ => Kind::Events,
            };
            ofds.push(Rc::new(RefCell::new(Ofd { kind, flags, path })));
        }
        self.shared.clear();
        for _ in 0..r.u32()? {
            let data = datas.get(r.u32()? as usize).cloned().flatten().ok_or("shared mapping of a file without contents")?;
            let mut pages = Vec::new();
            for _ in 0..r.u32()? {
                let p = unsafe { std::alloc::alloc_zeroed(std::alloc::Layout::from_size_align(4096, 4096).unwrap()) };
                unsafe { core::ptr::copy_nonoverlapping(r.take(4096)?.as_ptr(), p, 4096) };
                pages.push(p);
            }
            self.shared.push(SharedFile { data, pages });
        }
        for _ in 0..r.u32()? {
            let peer = r.u32()?;
            let fl = r.u8()?;
            let mut s = Sock { peer: (peer != u32::MAX).then_some(peer as usize), listening: fl & 1 != 0, closed: fl & 2 != 0, shut_wr: fl & 4 != 0, ..Default::default() };
            for _ in 0..r.u32()? { s.backlog.push_back(r.u32()? as usize); }
            for _ in 0..r.u32()? {
                let data = r.bytes()?.to_vec();
                let off = r.u32()? as usize;
                let mut fds = Vec::new();
                for _ in 0..r.u32()? { fds.push(ofds.get(r.u32()? as usize).ok_or("bad ofd id")?.clone()); }
                s.rx.push_back(Msg { data, off, fds });
            }
            self.socks.push(s);
        }
        let npool = r.u32()? as usize;
        let pool = r.take(npool * 4096)?;
        // the page contents live once, outside the processes, which refer to them copy-on-write (identical pages are shared)
        let lay = |n: usize| std::alloc::Layout::from_size_align(n.max(1) * 4096, 4096).unwrap();
        if self.snap_pool.0 != 0 { unsafe { std::alloc::dealloc(self.snap_pool.0 as *mut u8, lay(self.snap_pool.1)) }; }
        let base = unsafe { std::alloc::alloc(lay(npool)) };
        if base.is_null() { return Err("out of memory".into()); }
        unsafe { core::ptr::copy_nonoverlapping(pool.as_ptr(), base, pool.len()) };
        self.snap_pool = (base as usize, npool);
        for _ in 0..r.u32()? {
            let (pid, ppid, brk, brk_start, umask) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?);
            let exit = r.opt()?.map(|c| c as u32);
            let reaped = r.u8()? != 0;
            let (cwd, exe, cmd) = (r.str()?, r.str()?, r.str()?);
            let mut p = Proc { pid, ppid, mem: Mem::new_sparse(if exit.is_some() { 0x1000 } else { MEM_SIZE }), fds: HashMap::new(), cloexec: HashSet::new(), cwd, brk, brk_start,
                maps: BTreeMap::new(), umask, exe, cmd, sigact: HashMap::new(), threads: Vec::new(), cache: CacheSlot::default(), exit, reaped };
            for _ in 0..r.u32()? { let (s, e) = (r.u32()?, r.u32()?); p.maps.insert(s, e); }
            for _ in 0..r.u32()? { let s = r.u32()?; let mut a = [0u32; 5]; for v in a.iter_mut() { *v = r.u32()?; } p.sigact.insert(s, a); }
            for _ in 0..r.u32()? {
                let (fd, id, ce) = (r.u32()?, r.u32()? as usize, r.u8()? != 0);
                p.fds.insert(fd, ofds.get(id).ok_or("bad ofd id")?.clone());
                if ce { p.cloexec.insert(fd); }
            }
            for _ in 0..r.u32()? {
                let cpu = get_cpu(&mut r)?;
                let (tid, clear_tid) = (r.u32()?, r.u32()?);
                let (poll_deadline, wake_at) = (r.opt()?, r.opt()?);
                let state = match r.u8()? { 0 => St::Run, 1 => St::Futex(r.u32()?), 2 => St::Vfork(r.u32()? as usize), _ => St::Dead };
                p.threads.push(Thread { cpu, tid, poll_deadline, clear_tid, state, wake_at });
            }
            for _ in 0..r.u32()? {
                let (idx, v) = (r.u32()?, r.u32()?);
                if v == u32::MAX {
                    let (si, w) = (r.u32()? as usize, r.u32()?);
                    let ptr = *self.shared.get(si).and_then(|s| s.pages.get((w >> 1) as usize)).ok_or("bad shared page")?;
                    p.mem.map_ext(idx << 12, ptr, w & 1 != 0);
                    continue;
                }
                let id = (v >> 1) as usize;
                if id >= npool { return Err("bad page id".into()); }
                p.mem.map_cow(idx << 12, unsafe { base.add(id * 4096) }, v & 1 != 0);
            }
            self.procs.push(p);
        }
        self.windows.clear();
        for _ in 0..r.u32()? {
            let mut win = HostWin { hwnd: r.u32()?, owner: r.u32()?, style: r.u32()?, ex_style: r.u32()?, visible: r.u32()? != 0, sw: r.u32()?, sh: r.u32()?, ..Default::default() };
            for i in 0..4 { win.rect[i] = r.u32()? as i32; }
            for i in 0..4 { win.client[i] = r.u32()? as i32; }
            win.title = r.str()?;
            win.pixels = r.bytes()?.to_vec();
            self.windows.insert(win.hwnd, win);
        }
        self.zorder.clear();
        for _ in 0..r.u32()? { self.zorder.push(r.u32()?); }
        self.events.clear();
        self.blocked = false;
        Ok(())
    }
}
