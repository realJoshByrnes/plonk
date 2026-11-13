p='crates/host/src/linux.rs'
s=open(p,encoding='utf8',newline='').read()
k='            122 => { // uname'
add='''            99 | 100 | 268 | 269 => { // statfs/fstatfs(64): a roomy disk (4 GB, 2 GB free) so installers' free-space checks pass
                let buf = if nr >= 268 { c } else { b };
                let mut st = [0u8; 84];
                let put = |st: &mut [u8], off: usize, v: u64| st[off..off + 8].copy_from_slice(&v.to_le_bytes());
                st[0..4].copy_from_slice(&0xEF53u32.to_le_bytes());
                st[4..8].copy_from_slice(&4096u32.to_le_bytes());
                put(&mut st, 8, 1 << 20); put(&mut st, 16, 1 << 19); put(&mut st, 24, 1 << 19); put(&mut st, 32, 1 << 20); put(&mut st, 40, 1 << 19);
                st[56..60].copy_from_slice(&255u32.to_le_bytes());
                st[60..64].copy_from_slice(&4096u32.to_le_bytes());
                let _ = self.mem.write_bytes(buf, &st);
                Some(0)
            }
'''.replace('\n','\n')
assert k in s
s=s.replace(k,add+k,1)
open(p,'w',encoding='utf8',newline='').write(s)
