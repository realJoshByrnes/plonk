import re
p='crates/host/examples/linuxrun.rs'
s=open(p,encoding='utf8',newline='').read()
k='    if std::env::var("SAVE").is_ok() { // shut Wine'
s=s.replace(k,'    // EXPORT=file.tar: export ~/.wine immediately (no clean shutdown; files only)\n    if let Ok(f) = std::env::var("EXPORT") { std::fs::write(f, p.export_tar("/home/user/.wine")).unwrap(); }\n'+k,1)
open(p,'w',encoding='utf8',newline='').write(s)
