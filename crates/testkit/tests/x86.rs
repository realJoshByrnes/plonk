use plonk_cpu::*;
use plonk_testkit::*;

const ALLF: u32 = CF | ZF | SF | OF;

#[test]
fn snippets() {
    let cases = vec![
        Case::new("mov eax,imm", &[0xB8, 0x78, 0x56, 0x34, 0x12], &[], &[(EAX, 0x12345678)]),
        Case::new("add eax,ebx", &[0x01, 0xD8], &[(EAX, 2), (EBX, 3)], &[(EAX, 5)]).flags(ALLF, 0),
        Case::new("add carry", &[0x01, 0xD8], &[(EAX, 0xFFFF_FFFF), (EBX, 1)], &[(EAX, 0)]).flags(ALLF, CF | ZF),
        Case::new("add signed ovf", &[0x83, 0xC0, 0x01], &[(EAX, 0x7FFF_FFFF)], &[(EAX, 0x8000_0000)]).flags(ALLF, OF | SF),
        Case::new("sub borrow", &[0x29, 0xD8], &[(EAX, 3), (EBX, 5)], &[(EAX, 0xFFFF_FFFE)]).flags(ALLF, CF | SF),
        Case::new("cmp equal", &[0x39, 0xD8], &[(EAX, 5), (EBX, 5)], &[(EAX, 5)]).flags(ALLF, ZF),
        Case::new("xor self", &[0x31, 0xC0], &[(EAX, 0x1234)], &[(EAX, 0)]).flags(ALLF | PF, ZF | PF),
        Case::new("and eax,imm", &[0x25, 0xF0, 0, 0, 0], &[(EAX, 0xFF)], &[(EAX, 0xF0)]),
        Case::new("stc; inc keeps CF", &[0xF9, 0x40], &[(EAX, 0xFFFF_FFFF)], &[(EAX, 0)]).flags(CF | ZF, CF | ZF),
        Case::new("push/pop", &[0x50, 0x5B], &[(EAX, 0xCAFE)], &[(EBX, 0xCAFE), (ESP, STACK)]),
        Case::new("mov [ebx],eax", &[0xB8, 0xEF, 0xBE, 0xAD, 0xDE, 0xBB, 0, 0x20, 0, 0, 0x89, 0x03], &[], &[])
            .mem_exp(&[(0x2000, &[0xEF, 0xBE, 0xAD, 0xDE])]),
        Case::new("mov eax,[ebx+8]", &[0x8B, 0x43, 0x08], &[(EBX, 0x2000)], &[(EAX, 0x12345678)])
            .mem_init(&[(0x2008, &[0x78, 0x56, 0x34, 0x12])]),
        Case::new("mov eax,[ebx+ecx*4]", &[0x8B, 0x04, 0x8B], &[(EBX, 0x2000), (ECX, 2)], &[(EAX, 0x12345678)])
            .mem_init(&[(0x2008, &[0x78, 0x56, 0x34, 0x12])]),
        Case::new("lea", &[0x8D, 0x04, 0x8B], &[(EBX, 0x2000), (ECX, 2)], &[(EAX, 0x2008)]),
        Case::new("loop jnz", &[0x83, 0xC0, 0x02, 0x49, 0x75, 0xFA], &[(ECX, 3)], &[(EAX, 6), (ECX, 0)]),
        Case::new("call/ret", &[0xE8, 1, 0, 0, 0, 0xF4, 0xB8, 7, 0, 0, 0, 0xC3], &[], &[(EAX, 7), (ESP, STACK)]),
        Case::new("mov al", &[0xB0, 0x7F], &[(EAX, 0xAABB_CC00)], &[(EAX, 0xAABB_CC7F)]),
        Case::new("mov ah", &[0xB4, 0x11], &[(EAX, 0xAABB_CC00)], &[(EAX, 0xAABB_1100)]),
        Case::new("add al,al ovf", &[0x00, 0xC0], &[(EAX, 0x80)], &[(EAX, 0)]).flags(CF | ZF | OF, CF | ZF | OF),
        Case::new("test eax,eax", &[0x85, 0xC0], &[(EAX, 0)], &[(EAX, 0)]).flags(ZF | CF, ZF),
    ];
    assert_eq!(run_all(&cases), 0);
}

#[test]
fn unimplemented_reports_eip() {
    use plonk_mem::Mem;
    let mut m = Mem::new_owned(0x10000);
    m.w8(0x1000, 0x0E).unwrap(); // push cs: not implemented
    let mut c = Cpu::new();
    c.eip = 0x1000;
    assert_eq!(c.run(&mut m, 10), Exit::Unimplemented { eip: 0x1000, opcode: 0x0E });
}
