#!/usr/bin/env python3
"""Deterministic test-case generator for the hardware differential fuzzer.
usage: gen.py OUTDIR [seed]   -> OUTDIR/cases.bin, OUTDIR/templates.s
See README.md. Each template is GAS intel-syntax asm (or .byte lines)."""
import random, struct, sys, os

SCR = 512
BASE = 0x210000          # scratch buffer address (same on hardware and in the interpreter)
TABLE = 0x220000         # 256-dword CRC32 table
CF, PF, AF, ZF, SF, OF, DF = 1, 4, 0x10, 0x40, 0x80, 0x800, 0x400
ALL = CF | PF | AF | ZF | SF | OF | DF
FL_BITS = [CF, PF, AF, ZF, SF, OF, DF]
seed = int(sys.argv[2]) if len(sys.argv) > 2 else 12345
rng = random.Random(seed)
M32 = 0xFFFFFFFF
INT = [0, 1, 2, 3, 0x7f, 0x80, 0x81, 0xff, 0x100, 0x7fff, 0x8000, 0x8001, 0xffff, 0x10000,
       0x7fffffff, 0x80000000, 0x80000001, 0xfffffffe, 0xffffffff, 0xff00, 0xff0000, 0xff000000,
       0x55555555, 0xaaaaaaaa]
PTRS = {6: (128, 143), 7: (320, 335), 3: (224, 239)}   # esi, edi, ebx offsets into scratch


def rv():
    m = rng.random()
    if m < .4: return rng.getrandbits(32)
    if m < .8: return rng.choice(INT)
    if m < .9: return rng.choice(INT) ^ (1 << rng.randrange(32))
    return rng.randrange(0, 40)


class St:
    pass


TPL = []


def T(name, asm, mask=ALL, mem=False, ptr=None, init=None, mfn=None, n=None):
    if isinstance(asm, str): asm = asm.split(';')
    TPL.append(dict(name=name, asm='\n'.join(a.strip() for a in asm), mask=mask, mem=mem,
                    ptr=(ptr if ptr is not None else (PTRS if mem else {})), init=init, mfn=mfn,
                    n=n or (80 if mem else 100)))


def B(*b):
    return '.byte ' + ','.join('0x%02x' % (x & 0xff) for x in b)


def imm(v, nb):
    return [(v >> (8 * i)) & 0xff for i in range(nb)]


def mr(reg, rm): return 0xC0 | (reg << 3) | rm
def nz(i): return 7 if i == 4 else i   # avoid esp/sp


def set8(st, i, v):
    if i < 4: st.r[i] = (st.r[i] & ~0xff) | (v & 0xff)
    else: st.r[i - 4] = (st.r[i - 4] & ~0xff00) | ((v & 0xff) << 8)


def get8(st, i):
    return st.r[i] & 0xff if i < 4 else (st.r[i - 4] >> 8) & 0xff


def set16(st, i, v): st.r[i] = (st.r[i] & ~0xffff) | (v & 0xffff)
def off(st, reg): return st.r[reg] - BASE


def andn(*fs):
    def f(st):
        for g in fs: g(st)
    return f


CNT = [0, 1, 2, 3, 7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 255]


def cl_bias(lim=None):
    def f(st):
        v = rng.choice(CNT) if rng.random() < .8 else rng.getrandbits(8)
        if lim is not None: v = rng.choice([c for c in CNT if c <= lim])
        set8(st, 1, v)
    return f


def LOGIC(op): return op in ('and', 'or', 'xor', 'test')
OPS = ['add', 'or', 'adc', 'sbb', 'and', 'sub', 'xor', 'cmp']
SZ = {8: '', 16: '', 32: ''}
R32 = ['eax', 'ecx', 'edx', 'ebx', 'esp', 'ebp', 'esi', 'edi']
R16 = ['ax', 'cx', 'dx', 'bx', 'sp', 'bp', 'si', 'di']
R8 = ['al', 'cl', 'dl', 'bl', 'ah', 'ch', 'dh', 'bh']
RN = {8: R8, 16: R16, 32: R32}
PTR = {8: 'byte ptr', 16: 'word ptr', 32: 'dword ptr'}


def pre(sz): return [0x66] if sz == 16 else []
def maskop(op): return ALL & ~AF if LOGIC(op) else ALL


# ---------------------------------------------------------------- ALU
P8 = [(0, 1), (3, 3), (2, 5), (7, 0), (4, 6), (5, 2)]
P32 = [(0, 3), (1, 1), (2, 5), (7, 0), (3, 2), (5, 1)]
for oi, op in enumerate(OPS):
    for sz in (8, 16, 32):
        for (a, b) in (P8 if sz == 8 else P32):
            if sz != 8: a, b = nz(a), nz(b)
            w = 0 if sz == 8 else 1
            T(f'{op}{sz} rm,r {RN[sz][b]},{RN[sz][a]}', B(*pre(sz), oi * 8 + w, mr(a, b)), maskop(op))
            T(f'{op}{sz} r,rm {RN[sz][a]},{RN[sz][b]}', B(*pre(sz), oi * 8 + 2 + w, mr(a, b)), maskop(op))
        T(f'{op}{sz} [esi],ecx-form', B(*pre(sz), oi * 8 + (0 if sz == 8 else 1), 0x0E), maskop(op), mem=True)
        T(f'{op}{sz} r,[esi] edx-form', B(*pre(sz), oi * 8 + 2 + (0 if sz == 8 else 1), 0x16), maskop(op), mem=True)
        # acc, imm
        nb = sz // 8
        for k in range(3):
            v = rng.choice(INT + [rng.getrandbits(32)]) & ((1 << sz) - 1)
            T(f'{op}{sz} acc,imm {v:#x}', B(*pre(sz), oi * 8 + 4 + w, *imm(v, nb)), maskop(op))
        # group 1
        for rg in ((0, 4, 7) if sz == 8 else (0, 3, 5)):
            v = rng.choice(INT + [rng.getrandbits(32)]) & ((1 << sz) - 1)
            if sz == 8:
                T(f'{op}8 grp1 r{rg},imm8 {v:#x}', B(0x80, 0xC0 | oi << 3 | rg, v), maskop(op))
            else:
                T(f'{op}{sz} grp1 {RN[sz][rg]},imm {v:#x}', B(*pre(sz), 0x81, 0xC0 | oi << 3 | rg, *imm(v, sz // 8)), maskop(op))
                v8 = rng.choice([0, 1, 0x7f, 0x80, 0xff, 0xfe, 5, 0xf0])
                T(f'{op}{sz} grp1 {RN[sz][rg]},imm8s {v8:#x}', B(*pre(sz), 0x83, 0xC0 | oi << 3 | rg, v8), maskop(op))
        v = rng.getrandbits(sz)
        if sz == 8: T(f'{op}8 [esi],imm8', B(0x80, 0x06 | oi << 3, v), maskop(op), mem=True)
        else:
            T(f'{op}{sz} [esi],imm', B(*pre(sz), 0x81, 0x06 | oi << 3, *imm(v, sz // 8)), maskop(op), mem=True)
            T(f'{op}{sz} [esi],imm8s', B(*pre(sz), 0x83, 0x06 | oi << 3, rng.choice([1, 0x7f, 0x80, 0xff])), maskop(op), mem=True)

# carry-in chains
T('adc chain 64', 'add eax,ebx;adc edx,ecx')
T('sbb chain 64', 'sub eax,ebx;sbb edx,ecx')
T('stc adc0 x2', 'stc;adc eax,0;adc edx,0')
T('adc eax,eax', 'adc eax,eax;adc ecx,ecx')
T('sbb eax,eax', 'sbb eax,eax')
T('cmp sbb ecx,ecx', 'cmp eax,ebx;sbb ecx,ecx')
T('neg sbb', 'neg eax;sbb eax,eax')
T('add adc 16', 'add ax,bx;adc dx,cx')
T('add adc 8', 'add al,bl;adc ah,bh;adc cl,dl')
T('sub sbb 8', 'sub al,bl;sbb ah,bh')
T('shr rcr 64', 'shr edx,1;rcr eax,1', ALL & ~AF)
T('shl rcl 64', 'shl eax,1;rcl edx,1', ALL & ~AF)
T('add jc setc', 'add eax,ebx;setc cl;adc edx,0;sbb ebx,ebx')

# ---------------------------------------------------------------- inc dec neg not test
for sz in (8, 16, 32):
    for rg in ((0, 4, 7) if sz == 8 else (0, 3, 7)):
        n = RN[sz][rg]
        T(f'inc {n}', f'inc {n}'); T(f'dec {n}', f'dec {n}')
        T(f'neg {n}', f'neg {n}'); T(f'not {n}', f'not {n}')
        w = 0 if sz == 8 else 1
        T(f'inc/ff {n}', B(*pre(sz), 0xFE | w, 0xC0 | rg))
        T(f'dec/ff {n}', B(*pre(sz), 0xFE | w, 0xC8 | rg))
    for nm, md in (('inc', 0), ('dec', 1), ('not', 2), ('neg', 3)):
        w = 0 if sz == 8 else 1
        op = 0xFE | w if md < 2 else 0xF6 | w
        reg = md if md < 2 else md + 0  # F6 /2 not, /3 neg
        T(f'{nm}{sz} [esi]', B(*pre(sz), op, 0x06 | reg << 3), mem=True)
    for (a, b) in ((0, 1), (3, 3), (2, 7)):
        a, b = (a, b) if sz == 8 else (nz(a), nz(b))
        T(f'test{sz} {RN[sz][a]},{RN[sz][b]}', B(*pre(sz), 0x84 | (sz > 8), mr(a, b)), ALL & ~AF)
    for k in range(2):
        v = rng.choice(INT) & ((1 << sz) - 1)
        T(f'test{sz} acc,imm {v:#x}', B(*pre(sz), 0xA8 | (sz > 8), *imm(v, sz // 8)), ALL & ~AF)
        T(f'test{sz} F7/0 imm {v:#x}', B(*pre(sz), 0xF6 | (sz > 8), 0xC3 if sz > 8 or True else 0, *imm(v, sz // 8)), ALL & ~AF)
    T(f'test{sz} [esi],r', B(*pre(sz), 0x84 | (sz > 8), 0x0E), ALL & ~AF, mem=True)
T('inc eax short', B(0x40)); T('inc edi short', B(0x47)); T('dec ebp short', B(0x4d)); T('inc cx short', B(0x66, 0x41))
T('dec dx short', B(0x66, 0x4A))

# ---------------------------------------------------------------- mul / imul / div / idiv
MM = CF | OF | DF


def one_op(nm, sz, reg):
    return f'{nm} {RN[sz][reg]}'


for nm in ('mul', 'imul'):
    for sz, regs in ((8, (1, 3, 5, 0)), (16, (1, 3, 2)), (32, (1, 3, 2, 0))):
        for rg in regs:
            T(f'{nm}{sz} {RN[sz][rg]}', f'{nm} {RN[sz][rg]}', MM)
        T(f'{nm}{sz} [esi]', f'{nm} {PTR[sz]} [esi]', MM, mem=True)
for sz in (16, 32):
    for (a, b) in ((0, 1), (3, 5), (2, 2), (7, 0)):
        a, b = nz(a), nz(b)
        T(f'imul{sz} {RN[sz][a]},{RN[sz][b]}', f'imul {RN[sz][a]},{RN[sz][b]}', MM)
    T(f'imul{sz} r,[esi]', f'imul {RN[sz][1]},{PTR[sz]} [esi]', MM, mem=True)
    for k in range(4):
        v8 = rng.choice([0, 1, 2, 0x7f, 0x80, 0xff, 0x55, 0xf0])
        T(f'imul{sz} r,r,imm8 {v8:#x}', B(*pre(sz), 0x6B, mr(1, 3), v8), MM)
        v = rng.choice(INT) & ((1 << sz) - 1)
        T(f'imul{sz} r,r,imm {v:#x}', B(*pre(sz), 0x69, mr(2, 5), *imm(v, sz // 8)), MM)
    T(f'imul{sz} r,[esi],imm8', B(*pre(sz), 0x6B, 0x0E, 0x9B), MM, mem=True)


def nbits(w):
    return rng.getrandbits(rng.randint(1, w))


def init_div(w, signed, dreg, mem=False):
    def f(st):
        if signed:
            d = 0
            while d == 0:
                d = nbits(w - 1) * rng.choice([1, -1]) if w > 1 else 1
                d = max(-(1 << (w - 1)) + 1, min(d, (1 << (w - 1)) - 1)) if rng.random() < .9 else d
            q = nbits(w - 1) * rng.choice([1, -1])
            q = max(-(1 << (w - 1)), min(q, (1 << (w - 1)) - 1))
            r = rng.randint(0, abs(d) - 1)
            p = q * d
            sg = (1 if p > 0 else -1) if p != 0 else rng.choice([1, -1])
            D = p + sg * r
            if not (-(1 << (w - 1)) <= D // 1 < (1 << (w - 1)) or True): pass
            D &= (1 << (2 * w)) - 1
            dv = d & ((1 << w) - 1)
        else:
            d = max(1, nbits(w))
            q = nbits(w)
            r = rng.randint(0, d - 1)
            D = q * d + r
            dv = d
        if w == 8:
            st.r[0] = (st.r[0] & ~0xffff) | D
            if mem: st.buf[off(st, 6)] = dv
            else: set8(st, dreg, dv)
        elif w == 16:
            set16(st, 0, D & 0xffff); set16(st, 2, D >> 16)
            if mem: st.buf[off(st, 6):off(st, 6) + 2] = struct.pack('<H', dv)
            else: set16(st, dreg, dv)
        else:
            st.r[0] = D & M32; st.r[2] = D >> 32
            if mem: st.buf[off(st, 6):off(st, 6) + 4] = struct.pack('<I', dv)
            else: st.r[dreg] = dv
    return f


for nm in ('div', 'idiv'):
    for sz, regs in ((8, (1, 3, 7)), (16, (1, 3, 5)), (32, (1, 3, 5))):
        for rg in regs:
            T(f'{nm}{sz} {RN[sz][rg]}', f'{nm} {RN[sz][rg]}', 0, init=init_div(sz, nm == 'idiv', rg), mfn=lambda st: (DF, 0))
        T(f'{nm}{sz} [esi]', f'{nm} {PTR[sz]} [esi]', 0, mem=True, init=init_div(sz, nm == 'idiv', 0, True), mfn=lambda st: (DF, 0))
T('adler mod 65521', 'mov ecx,65521;xor edx,edx;div ecx', 0, mfn=lambda st: (DF, 0))

# ---------------------------------------------------------------- shifts / rotates
SHOPS = ['shl', 'shr', 'sar', 'rol', 'ror', 'rcl', 'rcr']


def shmask(op, size, c):
    c &= 31
    if c == 0: return ALL
    m = DF
    if op in ('shl', 'shr', 'sar'):
        m |= ZF | SF | PF | CF
        if c == 1: m |= OF
        if size < 32 and c > size and op != 'sar': m &= ~CF
    else:
        m |= CF | ZF | SF | PF | AF
        if c == 1: m |= OF
    return m


IMMS = [0, 1, 2, 3, 7, 8, 9, 15, 16, 17, 31, 32, 33]
for op in SHOPS:
    oi = SHOPS_I = {'rol': 0, 'ror': 1, 'rcl': 2, 'rcr': 3, 'shl': 4, 'shr': 5, 'sar': 7}[op]
    for sz in (8, 16, 32):
        w = 0 if sz == 8 else 1
        for rg in ((0, 4, 7, 1) if sz == 8 else (0, 2, 7, 1)):
            n = RN[sz][rg]
            T(f'{op}{sz} {n},1', B(*pre(sz), 0xD0 | w, 0xC0 | oi << 3 | rg), shmask(op, sz, 1), n=60)
            T(f'{op}{sz} {n},cl', B(*pre(sz), 0xD2 | w, 0xC0 | oi << 3 | rg), init=cl_bias(), mfn=(lambda op, sz: lambda st: (shmask(op, sz, get8(st, 1)), 0))(op, sz), n=80)
        for c in IMMS:
            rg = rng.choice((0, 4, 7) if sz == 8 else (0, 3, 5))
            T(f'{op}{sz} {RN[sz][rg]},{c}', B(*pre(sz), 0xC0 | w, 0xC0 | oi << 3 | rg, c), shmask(op, sz, c), n=50)
        T(f'{op}{sz} [esi],1', B(*pre(sz), 0xD0 | w, 0x06 | oi << 3), shmask(op, sz, 1), mem=True, n=40)
        T(f'{op}{sz} [esi],cl', B(*pre(sz), 0xD2 | w, 0x06 | oi << 3), mem=True, init=cl_bias(), mfn=(lambda op, sz: lambda st: (shmask(op, sz, get8(st, 1)), 0))(op, sz), n=60)
        T(f'{op}{sz} [esi],5', B(*pre(sz), 0xC0 | w, 0x06 | oi << 3, 5), shmask(op, sz, 5), mem=True, n=40)

for op, o1, o2 in (('shld', 0xA4, 0xA5), ('shrd', 0xAC, 0xAD)):
    for sz in (16, 32):
        lim = 16 if sz == 16 else None
        for c in ([1, 4, 8, 15, 16] if sz == 16 else [1, 4, 8, 16, 31]):
            T(f'{op}{sz} imm {c}', B(*pre(sz), 0x0F, o1, mr(2, 0), c), shmask('shl', 32, c), n=60)
        T(f'{op}{sz} cl', B(*pre(sz), 0x0F, o2, mr(2, 0)), init=cl_bias(lim),
          mfn=lambda st: (shmask('shl', 32, get8(st, 1)), 0), n=100)
        T(f'{op}{sz} cl ebx,edx', B(*pre(sz), 0x0F, o2, mr(2, 3)), init=cl_bias(lim),
          mfn=lambda st: (shmask('shl', 32, get8(st, 1)), 0), n=60)
        T(f'{op}{sz} [esi],edx,cl', B(*pre(sz), 0x0F, o2, 0x16), mem=True, init=cl_bias(lim),
          mfn=lambda st: (shmask('shl', 32, get8(st, 1)), 0), n=60)

# ---------------------------------------------------------------- bit ops
for nm, ro, io in (('bt', 0xA3, 4), ('bts', 0xAB, 5), ('btr', 0xB3, 6), ('btc', 0xBB, 7)):
    for sz in (16, 32):
        T(f'{nm}{sz} r,r', B(*pre(sz), 0x0F, ro, mr(1, 0)), CF | ZF | DF, n=100)
        T(f'{nm}{sz} r,r2', B(*pre(sz), 0x0F, ro, mr(2, 3)), CF | ZF | DF, n=100)
        for c in (0, 1, 7, 15, 16, 31, 32, 40):
            T(f'{nm}{sz} r,{c}', B(*pre(sz), 0x0F, 0xBA, 0xC0 | io << 3, c), CF | ZF | DF, n=40)

        def ibit(st):
            st.r[1] = rng.randint(-200, 200) & M32
        T(f'{nm}{sz} [esi],r', B(*pre(sz), 0x0F, ro, 0x0E), CF | ZF | DF, mem=True, init=ibit)
        T(f'{nm}{sz} [esi],imm', B(*pre(sz), 0x0F, 0xBA, 0x06 | io << 3, 13), CF | ZF | DF, mem=True)


def init_bs(sz):
    def f(st):
        r = rng.random()
        if r < .2: st.r[1] = st.r[1] & (~0xffff & M32) if sz == 16 else 0
        elif r < .6: st.r[1] = (1 << rng.randrange(sz)) | (rng.getrandbits(sz) & rng.getrandbits(sz) & ~0 if rng.random() < .5 else 0)
        st.r[1] &= M32
    return f


def mfn_bs(sz):
    return lambda st: (ZF | DF, 0 if (st.r[1] & ((1 << sz) - 1)) else 1)   # dest undefined if src zero


for nm, o in (('bsf', 0xBC), ('bsr', 0xBD)):
    for sz in (16, 32):
        T(f'{nm}{sz} r,r', B(*pre(sz), 0x0F, o, mr(0, 1)), init=init_bs(sz), mfn=mfn_bs(sz))
        T(f'{nm}{sz} r,r same', B(*pre(sz), 0x0F, o, mr(3, 3)), mfn=lambda st: (ZF | DF, 8 if (st.r[3] & 0xffff) == 0 else 0))
        T(f'{nm}{sz} r,[esi]', B(*pre(sz), 0x0F, o, 0x06), mem=True,
          mfn=(lambda sz: lambda st: (ZF | DF, 1 if int.from_bytes(st.buf[off(st, 6):off(st, 6) + sz // 8], 'little') == 0 else 0))(sz))
for rg in (0, 1, 3, 5, 7):
    T(f'bswap {R32[rg]}', f'bswap {R32[rg]}')

# ---------------------------------------------------------------- xchg cmpxchg xadd
for sz in (8, 16, 32):
    w = 0 if sz == 8 else 1
    for (a, b) in ((0, 1), (3, 3), (4, 2), (7, 5)) if sz == 8 else ((0, 1), (3, 3), (2, 7), (5, 0)):
        T(f'xchg{sz} {RN[sz][a]},{RN[sz][b]}', B(*pre(sz), 0x86 | w, mr(a, b)))
        T(f'xadd{sz} {RN[sz][b]},{RN[sz][a]}', B(*pre(sz), 0x0F, 0xC0 | w, mr(a, b)))

        def eqinit(st, a=a, b=b, sz=sz):
            if rng.random() < .4:
                if sz == 8: set8(st, b, get8(st, 0))
                elif sz == 16: set16(st, nz(b), st.r[0] & 0xffff)
                else: st.r[nz(b)] = st.r[0]
        T(f'cmpxchg{sz} {RN[sz][b]},{RN[sz][a]}', B(*pre(sz), 0x0F, 0xB0 | w, mr(a, b)), init=eqinit)
    T(f'xchg{sz} [esi],r', B(*pre(sz), 0x86 | w, 0x0E), mem=True)
    T(f'xadd{sz} [esi],r', B(*pre(sz), 0x0F, 0xC0 | w, 0x0E), mem=True)

    def eqm(st, sz=sz):
        if rng.random() < .4:
            o = off(st, 6)
            if sz == 8: st.buf[o] = st.r[0] & 0xff
            elif sz == 16: st.buf[o:o + 2] = struct.pack('<H', st.r[0] & 0xffff)
            else: st.buf[o:o + 4] = struct.pack('<I', st.r[0])
    T(f'cmpxchg{sz} [esi],r', B(*pre(sz), 0x0F, 0xB0 | (sz > 8), 0x0E), mem=True, init=eqm)
for rg in (1, 2, 3, 5, 7):
    T(f'xchg eax,{R32[rg]}', B(0x90 + rg)); T(f'xchg ax,{R16[rg]}', B(0x66, 0x90 + rg))
T('nop', 'nop')

# ---------------------------------------------------------------- movzx / movsx / cwde...
for nm, o8, o16 in (('movzx', 0xB6, 0xB7), ('movsx', 0xBE, 0xBF)):
    for d in (0, 2, 7):
        for s in range(8):
            if s in (1, 3, 4, 5) or d == 7 and s == 6:
                T(f'{nm} {R32[d]},{R8[s]}', f'{nm} {R32[d]},{R8[s]}')
                T(f'{nm} {R16[d]},{R8[s]}', f'{nm} {R16[d]},{R8[s]}')
        T(f'{nm} {R32[d]},{R16[nz(d ^ 1)]}', f'{nm} {R32[d]},{R16[nz(d ^ 1)]}')
    T(f'{nm} eax,byte [esi]', f'{nm} eax,byte ptr [esi]', mem=True)
    T(f'{nm} eax,word [esi]', f'{nm} eax,word ptr [esi]', mem=True)
    T(f'{nm} cx,byte [esi]', f'{nm} cx,byte ptr [esi]', mem=True)
T('cwde', 'cwde'); T('cdq', 'cdq'); T('cwd', 'cwd'); T('cbw', 'cbw')

# ---------------------------------------------------------------- setcc / cmovcc / jcc
CC = ['o', 'no', 'b', 'ae', 'e', 'ne', 'be', 'a', 's', 'ns', 'p', 'np', 'l', 'ge', 'le', 'g']
for i, c in enumerate(CC):
    T(f'set{c} al', f'set{c} al'); T(f'set{c} bh', f'set{c} bh'); T(f'set{c} [esi]', f'set{c} byte ptr [esi]', mem=True, n=30)
    T(f'cmov{c} eax,ebx', f'cmov{c} eax,ebx'); T(f'cmov{c} cx,dx', f'cmov{c} cx,dx')
    T(f'cmov{c} edx,[esi]', f'cmov{c} edx,dword ptr [esi]', mem=True, n=40)
    T(f'j{c} short', ['mov edx,1', B(0x70 + i, 5), 'mov edx,2'], n=40)
    T(f'j{c} near', ['mov edx,1', B(0x0F, 0x80 + i, 5, 0, 0, 0), 'mov edx,2'], n=40)
for nm, op in (('loopne', 0xE0), ('loope', 0xE1), ('loop', 0xE2), ('jecxz', 0xE3)):
    def cx_init(st):
        st.r[1] = rng.choice([0, 1, 2, 0x10000, 0x10001, 0xffff0000, 5, rng.getrandbits(32)])
    T(nm, B(op, 5) + '\nmov edx,2', init=cx_init, n=60)
T('jcxz (67)', B(0x67, 0xE3, 5) + '\nmov edx,2', init=lambda st: st.r.__setitem__(1, rng.choice([0, 1, 0x10000, 0x10001, 0xffff0000, 5, rng.getrandbits(32)])), n=60)
T('loop (67)', B(0x67, 0xE2, 5) + '\nmov edx,2', init=lambda st: st.r.__setitem__(1, rng.choice([0, 1, 0x10000, 0x10001, 0xffff0000, 5, rng.getrandbits(32)])), n=60)

# ---------------------------------------------------------------- lea
LEAS = ['[ebx]', '[ebx+8]', '[ebx-8]', '[ebx+0x12345678]', '[ebx+ecx]', '[ebx+ecx*2]', '[ebx+ecx*4+0x7f]', '[ebx+ecx*8-0x80]',
        '[ecx*8+0x1234]', '[ecx*2]', '[ebx+ecx*8+0x87654321]', '[esi+edi]', '[ebp+ecx*4]', '[ebp+0]', '[ebp]',
        '[0x12345678]', '[edx*4+edx]', '[edi+edi*2]', '[eax+eax*8+1]']
for a in LEAS:
    T(f'lea eax,{a}', f'lea eax,{a}')
    T(f'lea cx,{a}', f'lea cx,{a}', n=50)
T('lea ax,[bx+si+5] a16', 'lea ax,[bx+si+5]', n=50)
T('lea ax,[bp+di] a16', 'lea ax,[bp+di]', n=50)
T('lea eax,[bx] a16', 'lea eax,[bx+0x1234]', n=50)
T('lea esp-free eax,[eax+eax*4]', 'lea eax,[eax+eax*4]')

# ---------------------------------------------------------------- flags/misc
T('lahf', 'lahf'); T('sahf', 'sahf', ALL); T('clc', 'clc'); T('stc', 'stc'); T('cmc', 'cmc')
T('cld', 'cld'); T('std', 'std'); T('salc', B(0xD6)); T('xlat', 'xlatb', mem=True, ptr={3: (0, 200)})
T('pushfd/pop', 'pushfd;pop eax;and eax,0xCD5', DF)
T('popfd', 'and eax,0xCD5;push eax;popfd', ALL)
T('popfd or2', 'and eax,0xCD5;or eax,2;push eax;popfd;pushfd;pop edx;and edx,0xCD5', DF)
T('pushfd/popfd/cmc', 'pushfd;cmc;popfd')
T('daa', 'daa', ALL & ~OF); T('das', 'das', ALL & ~OF)
T('aaa', 'aaa', CF | AF | DF); T('aas', 'aas', CF | AF | DF)
T('aam 10', 'aam', ZF | SF | PF | DF); T('aam 7', B(0xD4, 7), ZF | SF | PF | DF)
T('aad 10', 'aad', ZF | SF | PF | DF); T('aad 7', B(0xD5, 7), ZF | SF | PF | DF)

# push / pop / leave / enter
T('push/pop regs', 'push eax;push ebx;pop ecx;pop edx')
T('push imm', 'push 0x12345678;push -1;pop eax;pop ecx')
T('push imm8/16', 'push 0x7f;.byte 0x66,0x68,0x34,0x12;pop ax;pop ecx')
T('push 16', 'push ax;push cx;pop dx;pop bx')
T('push/pop [esi]', 'push dword ptr [esi];pop dword ptr [edi]', mem=True)
T('pushad/popad', 'pushad;mov eax,0x11111111;mov ecx,0x22222222;popad')
T('pushad/pop', 'pushad;pop edi;pop esi;pop ebp;pop eax;pop ebx;pop edx;pop ecx;pop eax')
T('leave', 'push ebx;push ebp;mov ebp,esp;sub esp,16;leave;pop ecx', DF)
T('enter 8,0', 'enter 8,0;leave')
T('enter 0,0', 'enter 0,0;mov eax,ebp;leave', n=20)
T('call/ret', 'call 1f;mov edx,5;jmp 2f;1:mov edx,7;ret;2:')

# partial register writes / mov forms
for d, s in ((0, 4), (4, 0), (7, 2), (4, 1), (6, 7), (1, 5)):
    T(f'mov {R8[d]},{R8[s]}', f'mov {R8[d]},{R8[s]}')
T('mov ax,bx', 'mov ax,bx;mov dx,cx'); T('mov ah,al/al,ah', 'mov ah,al;mov bl,bh;mov ch,dl')
T('mov al,[esi]', 'mov al,[esi];mov ah,[esi+1];mov bl,[edi];mov dh,[edi+3]', mem=True)
T('mov [esi],ch', 'mov [esi],ch;mov [esi+1],ah;mov [edi],dl;mov [edi+2],bh', mem=True)
T('mov ax,[esi]', 'mov ax,[esi];mov cx,[edi]', mem=True)
T('mov [esi],ax', 'mov [esi],ax;mov [edi],cx', mem=True)
T('mov eax,imm; mov ah,imm', 'mov ah,0x7f;mov bl,0x80;mov ch,0xff;mov dx,0x1234;mov di,0xffff')
T('mov eax,[ebx+ecx*4+8]', 'mov eax,[ebx+ecx*4+8];mov edx,[ebx+ecx*2]', mem=True, ptr={3: (224, 239)},
  init=lambda st: st.r.__setitem__(1, rng.randrange(0, 16)))
T('mov [ebx+ecx*4],eax', 'mov [ebx+ecx*4],eax;mov [ebx+ecx*8+4],edx', mem=True, ptr={3: (224, 239)},
  init=lambda st: st.r.__setitem__(1, rng.randrange(0, 16)))
T('mov moffs', B(0xA1) + ',0x00,0x00,0x21,0x00\n' + B(0xA3) + ',0x40,0x00,0x21,0x00\n' + B(0xA0) + ',0x41,0x00,0x21,0x00', mem=True)
T('mov byte/word imm [esi]', 'mov byte ptr [esi],0x80;mov word ptr [esi+2],0xffff;mov dword ptr [esi+4],0x80000000', mem=True)
T('mov imm xor sub self', 'mov eax,5;xor ecx,ecx;sub edx,edx;or ebx,ebx;and edi,edi', ALL & ~AF)

# ---------------------------------------------------------------- string ops
def lowalpha(st):
    pass


def init_str(kind, sz, rep):
    def f(st):
        r = rng.random()
        st.r[1] = rng.randint(0, 3) if r < .3 else rng.randint(0, 20)
        n = sz // 8
        so, do = off(st, 6), off(st, 7)
        e = (st.r[1] + 2) * n
        if kind == 'cmps':
            if rng.random() < .85:
                st.buf[do - 90:do + 90] = st.buf[so - 90:so + 90]
                if rng.random() < .8:
                    k = rng.randint(-e, e)
                    st.buf[do + k] ^= 1 << rng.randrange(8)
        elif kind == 'scas':
            v = st.r[0] & ((1 << sz) - 1)
            pat = v.to_bytes(n, 'little')
            for k in range(-22, 23):
                st.buf[do + k * n:do + k * n + n] = pat
            if rng.random() < .8:
                k = rng.randint(-e, e)
                st.buf[do + k] ^= 1 << rng.randrange(8)
    return f


for kind, opc in (('movs', 0xA4), ('cmps', 0xA6), ('stos', 0xAA), ('lods', 0xAC), ('scas', 0xAE)):
    for sz, sfx in ((8, 'b'), (16, 'w'), (32, 'd')):
        for rep in ('', 'rep ', 'repe ', 'repne '):
            if kind in ('movs', 'stos', 'lods') and rep in ('repe ', 'repne '): continue
            if kind in ('cmps', 'scas') and rep == 'rep ': continue
            m = ALL if kind in ('cmps', 'scas') else ALL
            T(f'{rep}{kind}{sfx}', f'{rep}{kind}{sfx}', m, mem=True, init=init_str(kind, sz, rep), n=90 if rep else 40)

# ---------------------------------------------------------------- multi-instruction: CRC32 / inflate
def crc_init(st):
    st.r[1] = rng.randint(1, 24)
    st.r[0] = rng.getrandbits(32) if rng.random() < .7 else 0xFFFFFFFF


T('crc32 movzx/xor dl,al', 'mov ebp,ecx;1:movzx edx,byte ptr [esi];inc esi;xor dl,al;shr eax,8;xor eax,[edx*4+0x220000];dec ebp;jnz 1b;not eax',
  mem=True, init=crc_init, n=60)
T('crc32 bl via lodsb', 'mov ebp,ecx;mov edx,eax;1:lodsb;xor al,dl;movzx eax,al;shr edx,8;xor edx,[eax*4+0x220000];dec ebp;jnz 1b;not edx',
  mem=True, init=crc_init, n=60)
T('crc32 and ebx,0xff', 'mov ebp,ecx;1:mov ebx,eax;and ebx,0xff;xor bl,[esi];shr eax,8;xor eax,[ebx*4+0x220000];inc esi;dec ebp;jnz 1b',
  mem=True, ptr={6: (128, 143), 7: (320, 335)}, init=crc_init, n=60)
T('crc32 slice dword', 'mov ebp,ecx;1:xor eax,[esi];add esi,4;movzx edx,al;mov ebx,[edx*4+0x220000];movzx edx,ah;xor ebx,[edx*4+0x220000+0];shr eax,16;movzx edx,al;xor ebx,[edx*4+0x220000];shr eax,8;xor ebx,[eax*4+0x220000];mov eax,ebx;dec ebp;jnz 1b',
  mem=True, init=lambda st: (crc_init(st), st.r.__setitem__(1, rng.randint(1, 8))), n=60)
T('crc32 shr/ror inline', 'mov ebp,ecx;1:movzx edx,byte ptr [esi];inc esi;xor eax,edx;mov ecx,8;2:shr eax,1;jnc 3f;xor eax,0xEDB88320;3:dec ecx;jnz 2b;dec ebp;jnz 1b;mov ecx,ebp',
  mem=True, init=lambda st: (crc_init(st), st.r.__setitem__(1, rng.randint(1, 12))), n=60)
T('adler32', 'mov ebp,ecx;mov edx,eax;shr edx,16;movzx eax,ax;1:movzx ebx,byte ptr [esi];inc esi;add eax,ebx;add edx,eax;dec ebp;jnz 1b;mov ecx,65521;push edx;xor edx,edx;div ecx;mov ebx,edx;pop eax;xor edx,edx;div ecx;shl edx,16;or edx,ebx',
  mem=True, ptr={6: (128, 143)}, init=lambda st: (st.r.__setitem__(1, rng.randint(1, 30)), st.r.__setitem__(0, rng.getrandbits(32) & 0xfff0fff0)), n=60)


def inf_init(st):
    st.r[3] = rng.getrandbits(32) >> rng.randint(0, 31)
    st.r[1] = rng.randint(0, 7)


T('inflate getbits shl/or', 'mov ebp,12;xor edi,edi;2:mov edx,ebp;and edx,7;add edx,3;3:cmp ecx,edx;jae 4f;movzx eax,byte ptr [esi];inc esi;shl eax,cl;or ebx,eax;add ecx,8;jmp 3b;'
  '4:mov eax,1;push ecx;mov cl,dl;shl eax,cl;dec eax;pop ecx;and eax,ebx;push ecx;mov cl,dl;shr ebx,cl;pop ecx;sub ecx,edx;add edi,eax;rol edi,3;xor edi,eax;dec ebp;jnz 2b',
  mem=True, ptr={6: (128, 143)}, init=inf_init, n=80)
T('inflate rcr bitreader', 'mov ebp,24;xor eax,eax;1:shr bl,1;jnz 2f;mov bl,[esi];inc esi;rcr bl,1;2:adc eax,eax;dec ebp;jnz 1b',
  mem=True, ptr={6: (128, 143)}, init=lambda st: None, n=80)
T('inflate rcl bitreader edx', 'mov ebp,40;xor eax,eax;mov cl,1;1:add dl,dl;jnz 2f;mov dl,[esi];inc esi;adc dl,dl;2:rcl eax,1;dec ebp;jnz 1b',
  mem=True, ptr={6: (128, 143)}, n=80)
T('inflate shr edx,1 rcr', 'mov ebp,48;xor eax,eax;1:shr edx,1;jnz 2f;mov edx,[esi];add esi,4;stc;rcr edx,1;2:rcr eax,1;dec ebp;jnz 1b',
  mem=True, ptr={6: (128, 143)}, n=80)
T('inflate huff table xlat', 'mov ebp,16;xor edi,edi;1:lodsb;xlatb;movzx edx,al;add edi,edx;rol edi,5;dec ebp;jnz 1b',
  mem=True, ptr={6: (128, 143), 3: (0, 200)}, n=60)
T('inflate window copy', 'mov ecx,13;rep movsb;mov ecx,5;mov esi,edi;sub esi,3;rep movsb;mov ecx,6;rep movsd', mem=True, n=60)
T('inflate window copy df', 'mov ecx,7;std;rep movsb;cld;mov ecx,4;rep movsw', mem=True, n=40)
T('huff inc code', 'mov ebp,15;xor edx,edx;xor eax,eax;1:lea edx,[edx+edx];sub ebx,eax;sbb ecx,ecx;adc eax,eax;shl eax,1;cmp edx,eax;jb 2f;sub edx,eax;2:dec ebp;jnz 1b')
T('maxbits cmp/ja', 'cmp ecx,ebx;ja 1f;mov edx,1;1:cmp ax,dx;jbe 2f;mov dx,ax;2:movsx eax,al')
T('bit mask table build', 'mov cl,dl;and cl,31;mov eax,1;shl eax,cl;dec eax;and eax,ebx', ALL & ~AF)
T('byte swap idiom', 'xchg al,ah;rol eax,16;xchg al,ah', ALL & ~OF)
T('sbb/neg abs idiom', 'cdq;xor eax,edx;sub eax,edx')
T('mul64 idiom', 'mul ebx;add eax,ecx;adc edx,0')

# ---------------------------------------------------------------- emit
def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    # templates.s
    with open(os.path.join(out, 'templates.s'), 'w') as f:
        f.write('.intel_syntax noprefix\n.text\n')
        for i, t in enumerate(TPL):
            f.write(f'_t{i}:\n{t["asm"]}\n_t{i}e:\n')
        f.write('.data\n.globl _tpl_table\n_tpl_table:\n')
        for i in range(len(TPL)): f.write(f'.long _t{i},_t{i}e\n')
    ttab = bytes(i if c < 5 else 0 for i, c in zip(range(256), range(256)))
    alpha = b'\x00\x01\x7f\x80\xff'
    tbl = bytes(alpha[i % 5] for i in range(256))
    total = 0
    with open(os.path.join(out, 'cases.bin'), 'wb') as f:
        f.write(b'HWF1' + struct.pack('<I', len(TPL)))
        for t in TPL:
            nm = t['name'].encode()
            f.write(struct.pack('<I', len(nm)) + nm + struct.pack('<II', 1 if t['mem'] else 0, t['n']))
            for _ in range(t['n']):
                st = St()
                st.r = [rv() for _ in range(8)]
                fl = 0
                for b in FL_BITS:
                    if rng.random() < .5: fl |= b
                st.fl = fl
                st.buf = bytearray(rng.randbytes(SCR))
                if t['mem'] and rng.random() < .3: st.buf = bytearray(st.buf.translate(tbl))
                for reg, (lo, hi) in t['ptr'].items(): st.r[reg] = BASE + rng.randint(lo, hi)
                if t['init']: t['init'](st)
                mask, skip = t['mfn'](st) if t['mfn'] else (t['mask'], 0)
                f.write(struct.pack('<8IIII', *[x & M32 for x in st.r], st.fl, mask, skip))
                if t['mem']: f.write(bytes(st.buf))
                total += 1
    print(f'{len(TPL)} templates, {total} cases')


main()
