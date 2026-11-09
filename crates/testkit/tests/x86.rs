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
        Case::new("setne al", &[0x39, 0xD8, 0x0F, 0x95, 0xC0], &[(EAX, 0xFF00), (EBX, 1)], &[(EAX, 0xFF01)]),
        Case::new("movzx eax,bl", &[0x0F, 0xB6, 0xC3], &[(EBX, 0x1280)], &[(EAX, 0x80)]),
        Case::new("movsx eax,bl", &[0x0F, 0xBE, 0xC3], &[(EBX, 0x1280)], &[(EAX, 0xFFFF_FF80)]),
        Case::new("movsx eax,bx", &[0x0F, 0xBF, 0xC3], &[(EBX, 0x1_8000)], &[(EAX, 0xFFFF_8000)]),
        Case::new("call [mem]", &[0xFF, 0x15, 0x00, 0x20, 0, 0, 0xF4], &[], &[(ESP, STACK - 4)])
            .mem_init(&[(0x2000, &[0x06, 0x10, 0, 0])]).mem_exp(&[(STACK - 4, &[0x06, 0x10, 0, 0])]),
        Case::new("inc dword [ebx]", &[0xFF, 0x03], &[(EBX, 0x2000)], &[]).mem_init(&[(0x2000, &[0xFF, 0, 0, 0])]).mem_exp(&[(0x2000, &[0, 1, 0, 0])]),
        Case::new("push [ebx]", &[0xFF, 0x33, 0x58], &[(EBX, 0x2000)], &[(EAX, 0x1234)]).mem_init(&[(0x2000, &[0x34, 0x12, 0, 0])]),
        Case::new("add ax,bx", &[0x66, 0x01, 0xD8], &[(EAX, 0xFFFF_0001), (EBX, 0xFFFF)], &[(EAX, 0xFFFF_0000)]).flags(CF | ZF, CF | ZF),
        Case::new("mov ax,imm16", &[0x66, 0xB8, 0x34, 0x12], &[(EAX, 0xAAAA_0000)], &[(EAX, 0xAAAA_1234)]),
        Case::new("shl eax,4", &[0xC1, 0xE0, 0x04], &[(EAX, 0x1234_5678)], &[(EAX, 0x2345_6780)]).flags(CF, CF),
        Case::new("shr eax,1", &[0xD1, 0xE8], &[(EAX, 3)], &[(EAX, 1)]).flags(CF, CF),
        Case::new("sar eax,4", &[0xC1, 0xF8, 0x04], &[(EAX, 0x8000_0000)], &[(EAX, 0xF800_0000)]),
        Case::new("rol eax,8", &[0xC1, 0xC0, 0x08], &[(EAX, 0x1234_5678)], &[(EAX, 0x3456_7812)]),
        Case::new("ror eax,8", &[0xC1, 0xC8, 0x08], &[(EAX, 0x1234_5678)], &[(EAX, 0x7812_3456)]),
        Case::new("mul ebx", &[0xF7, 0xE3], &[(EAX, 0x10000), (EBX, 0x10000)], &[(EAX, 0), (EDX, 1)]).flags(CF | OF, CF | OF),
        Case::new("imul eax,ebx", &[0x0F, 0xAF, 0xC3], &[(EAX, 0xFFFF_FFFD), (EBX, 4)], &[(EAX, 0xFFFF_FFF4)]).flags(CF | OF, 0),
        Case::new("imul eax,ebx,10", &[0x6B, 0xC3, 0x0A], &[(EBX, 7)], &[(EAX, 70)]),
        Case::new("div ebx", &[0xF7, 0xF3], &[(EAX, 100), (EBX, 7)], &[(EAX, 14), (EDX, 2)]),
        Case::new("idiv ebx", &[0xF7, 0xFB], &[(EAX, 0xFFFF_FF9C), (EDX, 0xFFFF_FFFF), (EBX, 7)], &[(EAX, 0xFFFF_FFF2), (EDX, 0xFFFF_FFFE)]),
        Case::new("div bl", &[0xF6, 0xF3], &[(EAX, 100), (EBX, 7)], &[(EAX, 0x020E)]),
        Case::new("cdq", &[0x99], &[(EAX, 0x8000_0000)], &[(EDX, 0xFFFF_FFFF)]),
        Case::new("neg eax", &[0xF7, 0xD8], &[(EAX, 5)], &[(EAX, 0xFFFF_FFFB)]).flags(CF, CF),
        Case::new("not eax", &[0xF7, 0xD0], &[], &[(EAX, 0xFFFF_FFFF)]),
        Case::new("rep stosd", &[0xF3, 0xAB], &[(EDI, 0x2000), (ECX, 3), (EAX, 0x1111_1111)], &[(ECX, 0), (EDI, 0x200C)])
            .mem_exp(&[(0x2000, &[0x11; 12])]),
        Case::new("rep movsb", &[0xF3, 0xA4], &[(ESI, 0x2000), (EDI, 0x3000), (ECX, 4)], &[(ECX, 0)])
            .mem_init(&[(0x2000, &[1, 2, 3, 4])]).mem_exp(&[(0x3000, &[1, 2, 3, 4])]),
        Case::new("repe cmpsb", &[0xF3, 0xA6], &[(ESI, 0x2000), (EDI, 0x3000), (ECX, 4)], &[(ECX, 1)]).flags(ZF, 0)
            .mem_init(&[(0x2000, &[1, 2, 3, 4]), (0x3000, &[1, 2, 9, 4])]),
        Case::new("repne scasb", &[0xF2, 0xAE], &[(EDI, 0x2000), (ECX, 10), (EAX, 3)], &[(EDI, 0x2003), (ECX, 7)]).flags(ZF, ZF)
            .mem_init(&[(0x2000, &[1, 2, 3])]),
        Case::new("lodsb", &[0xAC], &[(ESI, 0x2000)], &[(EAX, 0x7F), (ESI, 0x2001)]).mem_init(&[(0x2000, &[0x7F])]),
        Case::new("xchg eax,ebx", &[0x93], &[(EAX, 1), (EBX, 2)], &[(EAX, 2), (EBX, 1)]),
        Case::new("cmovne", &[0x39, 0xD8, 0x0F, 0x45, 0xC1], &[(EAX, 1), (EBX, 2), (ECX, 9)], &[(EAX, 9)]),
        Case::new("bsf", &[0x0F, 0xBC, 0xC3], &[(EBX, 0x10)], &[(EAX, 4)]).flags(ZF, 0),
        Case::new("bswap", &[0x0F, 0xC8], &[(EAX, 0x1122_3344)], &[(EAX, 0x4433_2211)]),
        Case::new("bt eax,3", &[0x0F, 0xBA, 0xE0, 0x03], &[(EAX, 8)], &[]).flags(CF, CF),
        Case::new("leave", &[0xC9], &[(EBP, 0x7000)], &[(EBP, 0x1234_5678), (ESP, 0x7004)]).mem_init(&[(0x7000, &[0x78, 0x56, 0x34, 0x12])]),
        Case::new("pushf/pop", &[0x9C, 0x58], &[], &[(EAX, 0x202)]),
        Case::new("ret 4", &[0xC2, 0x04, 0x00], &[(ESP, 0x7000)], &[(ESP, 0x7008)]).mem_init(&[(0x7000, &[0x03, 0x10, 0, 0])]),
        Case::new("shld", &[0x0F, 0xA4, 0xD8, 0x04], &[(EAX, 0x1234_5678), (EBX, 0x9ABC_DEF0)], &[(EAX, 0x2345_6789)]),
        Case::new("cmpxchg", &[0x0F, 0xB1, 0xCB], &[(EAX, 5), (EBX, 5), (ECX, 9)], &[(EBX, 9)]).flags(ZF, ZF),
        Case::new("xadd", &[0x0F, 0xC1, 0xD8], &[(EAX, 1), (EBX, 2)], &[(EAX, 3), (EBX, 1)]),
        Case::new("stc; adc", &[0xF9, 0x11, 0xD8], &[(EAX, 1), (EBX, 2)], &[(EAX, 4)]),
        Case::new("stc; sbb", &[0xF9, 0x19, 0xD8], &[(EAX, 5), (EBX, 2)], &[(EAX, 2)]),
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

#[test]
fn fs_segment_and_div_error() {
    use plonk_mem::Mem;
    let mut m = Mem::new_owned(0x10000);
    m.write_bytes(0x1000, &[0x64, 0xA1, 0x18, 0, 0, 0, 0xF4]).unwrap(); // mov eax, fs:[0x18]
    m.w32(0x3018, 0xCAFE).unwrap();
    let mut c = Cpu::new();
    c.eip = 0x1000;
    c.fs_base = 0x3000;
    assert_eq!(c.run(&mut m, 10), Exit::Halt);
    assert_eq!(c.r[EAX], 0xCAFE);
    m.write_bytes(0x1000, &[0x31, 0xDB, 0xF7, 0xF3]).unwrap(); // xor ebx,ebx; div ebx
    c.eip = 0x1000;
    assert_eq!(c.run(&mut m, 10), Exit::DivideError(0x1002));
}
