#include <windows.h>
#include <stdio.h>

static volatile LONG seen[3];
static LONG CALLBACK handler(EXCEPTION_POINTERS *ep) {
    DWORD c = ep->ExceptionRecord->ExceptionCode;
    if (c == EXCEPTION_ACCESS_VIOLATION) { seen[0] = (LONG)ep->ExceptionRecord->ExceptionInformation[1]; ep->ContextRecord->Eip += 10; return EXCEPTION_CONTINUE_EXECUTION; }
    if (c == EXCEPTION_INT_DIVIDE_BY_ZERO) { seen[1] = 1; ep->ContextRecord->Eip += 2; return EXCEPTION_CONTINUE_EXECUTION; }
    if (c == 0xE0000001) { seen[2] = (LONG)ep->ExceptionRecord->ExceptionInformation[0]; return EXCEPTION_CONTINUE_EXECUTION; }
    return EXCEPTION_CONTINUE_SEARCH;
}

int main(void) {
    AddVectoredExceptionHandler(1, handler);
    __asm__ volatile("movl $0, 0x10");                                         /* 10-byte store to an unmapped page */
    printf("av addr=%lx\n", (unsigned long)seen[0]);
    __asm__ volatile("xorl %%ecx,%%ecx\n\tdivl %%ecx" ::: "eax", "edx", "ecx"); /* the divl is 2 bytes */
    printf("div=%ld\n", (long)seen[1]);
    ULONG_PTR arg = 42; RaiseException(0xE0000001, 0, 1, &arg);
    printf("raise arg=%ld\n", (long)seen[2]);
    printf("end\n");
    return 0;
}
