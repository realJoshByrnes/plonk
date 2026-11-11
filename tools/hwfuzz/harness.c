/* Hardware side of the differential fuzzer (32-bit mingw).
   harness.exe cases.bin hw.bin code.bin
   Runs every template/case natively; writes results and the template machine code. */
#include <windows.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#define REGION 0x200000u
#define BASE   0x210000u
#define TABLE  0x220000u
#define CODE   0x230000u
#define SCR    512

uint32_t inr[8], inflags, outr[8], outfl, outesp, saved_esp, start_esp, faulted, in_run;
void *codeptr;
extern uint32_t tpl_table[];
void run_native(void);
void capture(void);

__asm__(
".text\n.intel_syntax noprefix\n"
".globl _run_native\n_run_native:\n"
" pushad\n"
" mov dword ptr [_saved_esp], esp\n"
" push dword ptr [_inflags]\n"
" popfd\n"
" mov dword ptr [_start_esp], esp\n"
" mov eax, dword ptr [_inr+0]\n mov ecx, dword ptr [_inr+4]\n mov edx, dword ptr [_inr+8]\n"
" mov ebx, dword ptr [_inr+12]\n mov ebp, dword ptr [_inr+20]\n mov esi, dword ptr [_inr+24]\n"
" mov edi, dword ptr [_inr+28]\n"
" jmp dword ptr [_codeptr]\n"
".globl _capture\n_capture:\n"
" mov dword ptr [_outr+0], eax\n mov dword ptr [_outr+4], ecx\n mov dword ptr [_outr+8], edx\n"
" mov dword ptr [_outr+12], ebx\n mov dword ptr [_outr+20], ebp\n mov dword ptr [_outr+24], esi\n"
" mov dword ptr [_outr+28], edi\n mov dword ptr [_outesp], esp\n"
" pushfd\n pop dword ptr [_outfl]\n"
" mov esp, dword ptr [_saved_esp]\n cld\n popad\n ret\n"
".att_syntax\n");

static LONG CALLBACK veh(PEXCEPTION_POINTERS p) {
    if (!in_run) return EXCEPTION_CONTINUE_SEARCH;
    faulted = 1;
    p->ContextRecord->Eip = (DWORD)(uintptr_t)capture;
    return EXCEPTION_CONTINUE_EXECUTION;
}

static uint32_t rd32(FILE *f) { uint32_t v; if (fread(&v, 4, 1, f) != 1) { fprintf(stderr, "short read\n"); exit(2); } return v; }

int main(int argc, char **argv) {
    if (argc < 4) { fprintf(stderr, "usage: harness cases.bin hw.bin code.bin\n"); return 1; }
    uint8_t *reg = VirtualAlloc((void *)REGION, 0x40000, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE);
    if (reg != (uint8_t *)REGION) { fprintf(stderr, "cannot map fixed region at %x\n", REGION); return 1; }
    for (int i = 0; i < 256; i++) {
        uint32_t c = i;
        for (int k = 0; k < 8; k++) c = (c & 1) ? (c >> 1) ^ 0xEDB88320u : c >> 1;
        ((uint32_t *)TABLE)[i] = c;
    }
    AddVectoredExceptionHandler(1, veh);
    FILE *fc = fopen(argv[1], "rb"), *fh = fopen(argv[2], "wb"), *fk = fopen(argv[3], "wb");
    if (!fc || !fh || !fk) { fprintf(stderr, "open failed\n"); return 1; }
    char magic[4]; fread(magic, 1, 4, fc);
    uint32_t nt = rd32(fc);
    fwrite(&nt, 4, 1, fk);
    uint8_t *scr = (uint8_t *)BASE;
    uint8_t buf[SCR];
    uint32_t nfault = 0, ncase = 0;
    for (uint32_t t = 0; t < nt; t++) {
        uint32_t nl = rd32(fc); fseek(fc, nl, SEEK_CUR);
        uint32_t mem = rd32(fc), n = rd32(fc);
        uint32_t s = tpl_table[t * 2], e = tpl_table[t * 2 + 1], len = e - s;
        fwrite(&len, 4, 1, fk); fwrite((void *)s, 1, len, fk);
        memcpy((void *)CODE, (void *)s, len);
        uint8_t *tail = (uint8_t *)CODE + len;
        tail[0] = 0xE9; *(int32_t *)(tail + 1) = (int32_t)((uint8_t *)capture - (tail + 5));
        FlushInstructionCache(GetCurrentProcess(), (void *)CODE, 64);
        for (uint32_t i = 0; i < n; i++) {
            uint32_t rec[11];
            if (fread(rec, 4, 11, fc) != 11) { fprintf(stderr, "short cases\n"); return 2; }
            if (mem) { if (fread(buf, 1, SCR, fc) != SCR) return 2; memcpy(scr, buf, SCR); }
            memcpy(inr, rec, 32);
            inflags = (rec[8] & 0xCD5) | 0x202;
            codeptr = (void *)CODE; faulted = 0; in_run = 1;
            run_native();
            in_run = 0;
            outr[4] = outesp - start_esp;
            uint32_t o[11];
            o[0] = faulted; memcpy(o + 1, outr, 32); o[9] = outfl;
            fwrite(o, 4, 10, fh);
            if (mem) fwrite(scr, 1, SCR, fh);
            nfault += faulted; ncase++;
        }
    }
    fclose(fh); fclose(fk);
    printf("hardware: %u cases, %u faulted\n", ncase, nfault);
    return 0;
}
