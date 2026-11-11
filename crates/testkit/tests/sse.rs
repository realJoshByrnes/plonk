use plonk_cpu::*;
use plonk_testkit::*;


#[test]
fn sse_snippets() {
    let cases = vec![
        // addps xmm0,xmm1 / mulps / subps / divps
        Case::new("addps", &[0x0F, 0x58, 0xC1], &[], &[]).xmm(&[(0, 0x3F80_0000_4000_0000_3F80_0000_3F80_0000), (1, 0x4000_0000_4000_0000_4000_0000_4000_0000)], &[(0, 0x4040_0000_4080_0000_4040_0000_4040_0000)]),
        Case::new("mulss keeps upper", &[0xF3, 0x0F, 0x59, 0xC1], &[], &[]).xmm(&[(0, 0x1111_1111_2222_2222_3333_3333_4000_0000), (1, 0x4040_0000)], &[(0, 0x1111_1111_2222_2222_3333_3333_40C0_0000)]),
        Case::new("addsd", &[0xF2, 0x0F, 0x58, 0xC1], &[], &[]).xmm(&[(0, 0x7777_7777_7777_7777_3FF0_0000_0000_0000), (1, 0x4000_0000_0000_0000)], &[(0, 0x7777_7777_7777_7777_4008_0000_0000_0000)]),
        Case::new("sqrtpd", &[0x66, 0x0F, 0x51, 0xC1], &[], &[]).xmm(&[(1, 0x4010_0000_0000_0000_4000_0000_0000_0000 + 0x0)], &[(0, 0x4000_0000_0000_0000_3FF6_A09E_667F_3BCD)]),
        Case::new("xorps self", &[0x0F, 0x57, 0xC0], &[], &[]).xmm(&[(0, 0x1234)], &[(0, 0)]),
        Case::new("andnps", &[0x0F, 0x55, 0xC1], &[], &[]).xmm(&[(0, 0xFF00), (1, 0xFFFF)], &[(0, 0x00FF)]),
        // movaps load from memory then store
        Case::new("movaps load", &[0x0F, 0x28, 0x03], &[(EBX, 0x2000)], &[]).mem_init(&[(0x2000, &[1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0])]).xmm(&[], &[(0, 0x0000_0004_0000_0003_0000_0002_0000_0001)]),
        Case::new("movups store", &[0x0F, 0x11, 0x03], &[(EBX, 0x2000)], &[]).xmm(&[(0, 0x0000_0004_0000_0003_0000_0002_0000_0001)], &[]).mem_exp(&[(0x2000, &[1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0])]),
        Case::new("movss load zeroes upper", &[0xF3, 0x0F, 0x10, 0x03], &[(EBX, 0x2000)], &[]).mem_init(&[(0x2000, &[0, 0, 0x80, 0x3F])]).xmm(&[(0, u128::MAX)], &[(0, 0x3F80_0000)]),
        Case::new("movd xmm,eax", &[0x66, 0x0F, 0x6E, 0xC0], &[(EAX, 0xDEADBEEF)], &[]).xmm(&[(0, u128::MAX)], &[(0, 0xDEAD_BEEF)]),
        Case::new("movd eax,xmm", &[0x66, 0x0F, 0x7E, 0xC0], &[], &[(EAX, 0x1234_5678)]).xmm(&[(0, 0xAAAA_0000_1234_5678)], &[]),
        Case::new("movq xmm,xmm", &[0xF3, 0x0F, 0x7E, 0xC1], &[], &[]).xmm(&[(1, 0xFFFF_FFFF_FFFF_FFFF_0123_4567_89AB_CDEF)], &[(0, 0x0123_4567_89AB_CDEF)]),
        Case::new("movhlps", &[0x0F, 0x12, 0xC1], &[], &[]).xmm(&[(0, 0), (1, 0x99 << 64 | 0x11)], &[(0, 0x99)]),
        // conversions
        Case::new("cvtsi2sd", &[0xF2, 0x0F, 0x2A, 0xC0], &[(EAX, 5)], &[]).xmm(&[(0, 0)], &[(0, 0x4014_0000_0000_0000)]),
        Case::new("cvttsd2si", &[0xF2, 0x0F, 0x2C, 0xC1], &[], &[(EAX, 3)]).xmm(&[(1, 0x400E_0000_0000_0000)], &[]), // 3.75 -> 3
        Case::new("cvtsd2si rounds", &[0xF2, 0x0F, 0x2D, 0xC1], &[], &[(EAX, 4)]).xmm(&[(1, 0x400E_0000_0000_0000)], &[]), // 3.75 -> 4
        Case::new("cvtss2sd", &[0xF3, 0x0F, 0x5A, 0xC1], &[], &[]).xmm(&[(1, 0x3FC0_0000)], &[(0, 0x3FF8_0000_0000_0000)]),
        Case::new("cvtsd2ss", &[0xF2, 0x0F, 0x5A, 0xC1], &[], &[]).xmm(&[(1, 0x3FF8_0000_0000_0000)], &[(0, 0x3FC0_0000)]),
        Case::new("cvtdq2ps", &[0x0F, 0x5B, 0xC1], &[], &[]).xmm(&[(1, 0x0000_0002_0000_0001)], &[(0, 0x0000_0000_0000_0000_4000_0000_3F80_0000)]),
        // comparisons
        Case::new("ucomisd less", &[0x66, 0x0F, 0x2E, 0xC1], &[], &[]).xmm(&[(0, 0x3FF0_0000_0000_0000), (1, 0x4000_0000_0000_0000)], &[]).flags(ZF | CF | PF, CF),
        Case::new("comiss equal", &[0x0F, 0x2F, 0xC1], &[], &[]).xmm(&[(0, 0x3F80_0000), (1, 0x3F80_0000)], &[]).flags(ZF | CF | PF, ZF),
        Case::new("cmpltps", &[0x0F, 0xC2, 0xC1, 0x01], &[], &[]).xmm(&[(0, 0x40A0_0000_3F80_0000_40A0_0000_3F80_0000), (1, 0x4000_0000_4000_0000_4000_0000_4000_0000)], &[(0, 0x0000_0000_FFFF_FFFF_0000_0000_FFFF_FFFF)]),
        // integer SSE2
        Case::new("paddd", &[0x66, 0x0F, 0xFE, 0xC1], &[], &[]).xmm(&[(0, 0x0000_0002_0000_0001), (1, 0x0000_0002_0000_0001)], &[(0, 0x0000_0004_0000_0002)]),
        Case::new("psubb sat", &[0x66, 0x0F, 0xD8, 0xC1], &[], &[]).xmm(&[(0, 0x0501), (1, 0x0302)], &[(0, 0x0200)]),
        Case::new("pxor self", &[0x66, 0x0F, 0xEF, 0xC0], &[], &[]).xmm(&[(0, 77)], &[(0, 0)]),
        Case::new("pcmpeqb", &[0x66, 0x0F, 0x74, 0xC1], &[], &[]).xmm(&[(0, 0x0102), (1, 0x0103)], &[(0, 0xFF_00 | (u128::MAX << 16))]),
        Case::new("pmovmskb", &[0x66, 0x0F, 0xD7, 0xC1], &[], &[(EAX, 0x8001)]).xmm(&[(1, 0x80 << 120 | 0x80)], &[]),
        Case::new("pshufd", &[0x66, 0x0F, 0x70, 0xC1, 0x1B], &[], &[]).xmm(&[(1, 0x0000_0004_0000_0003_0000_0002_0000_0001)], &[(0, 0x0000_0001_0000_0002_0000_0003_0000_0004)]),
        Case::new("punpcklbw", &[0x66, 0x0F, 0x60, 0xC1], &[], &[]).xmm(&[(0, 0x0201), (1, 0x0403)], &[(0, 0x0402_0301)]),
        Case::new("psllq imm", &[0x66, 0x0F, 0x73, 0xF0, 0x04], &[], &[]).xmm(&[(0, 0xF)], &[(0, 0xF0)]),
        Case::new("psrldq imm", &[0x66, 0x0F, 0x73, 0xD8, 0x08], &[], &[]).xmm(&[(0, 0x5 << 64 | 0x7)], &[(0, 0x5)]),
        Case::new("pmuludq", &[0x66, 0x0F, 0xF4, 0xC1], &[], &[]).xmm(&[(0, 0xFFFF_FFFF), (1, 0x10)], &[(0, 0xF_FFFF_FFF0)]),
        Case::new("pmaddwd", &[0x66, 0x0F, 0xF5, 0xC1], &[], &[]).xmm(&[(0, 0x0003_0002), (1, 0x0005_0004)], &[(0, 23)]),
        Case::new("packuswb", &[0x66, 0x0F, 0x67, 0xC1], &[], &[]).xmm(&[(0, 0x00FF_0100_FFFF_0001), (1, 0)], &[(0, 0xFFFF_0001)]),
        // state
        Case::new("stmxcsr", &[0x0F, 0xAE, 0x1B], &[(EBX, 0x2000)], &[]).mem_exp(&[(0x2000, &[0x80, 0x1F, 0, 0])]),
    ];
    // two of the above were written awkwardly; failures print the case name so they are easy to fix
    let n = run_all(&cases);
    assert_eq!(n, 0);
}
