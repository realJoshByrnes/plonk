#include <windows.h>
#include <process.h>
#include <stdio.h>

static CRITICAL_SECTION cs;
static volatile LONG counter = 0, total = 0;
static HANDLE ev_go, ev_done;
static DWORD tls;

static DWORD WINAPI worker(LPVOID arg) {
    int id = (int)(INT_PTR)arg;
    TlsSetValue(tls, (LPVOID)(INT_PTR)(100 + id));
    for (int i = 0; i < 1000; i++) {
        EnterCriticalSection(&cs); counter++; LeaveCriticalSection(&cs);
        InterlockedIncrement(&total);
    }
    Sleep(20 * id);
    return 10 + (int)(INT_PTR)TlsGetValue(tls) - 100 + id;   /* 10 + 0 + id ... = 10 + id */
}

static unsigned __stdcall waiter(void *arg) {
    WaitForSingleObject(ev_go, INFINITE);       /* blocks until main signals */
    SetEvent(ev_done);
    return 7;
}

int main(void) {
    InitializeCriticalSection(&cs);
    tls = TlsAlloc();
    HANDLE th[3];
    for (int i = 0; i < 3; i++) th[i] = CreateThread(0, 0, worker, (LPVOID)(INT_PTR)(i + 1), 0, 0);
    DWORD w = WaitForMultipleObjects(3, th, TRUE, 5000);
    DWORD ec[3]; for (int i = 0; i < 3; i++) GetExitCodeThread(th[i], &ec[i]);
    printf("wait=%lu counter=%ld total=%ld exits=%lu,%lu,%lu\n", (unsigned long)w, (long)counter, (long)total, (unsigned long)ec[0], (unsigned long)ec[1], (unsigned long)ec[2]);
    printf("main tls=%d\n", (int)(INT_PTR)TlsGetValue(tls));

    ev_go = CreateEventA(0, TRUE, FALSE, 0); ev_done = CreateEventA(0, TRUE, FALSE, 0);
    HANDLE t2 = (HANDLE)_beginthreadex(0, 0, waiter, 0, 0, 0);
    Sleep(30);
    printf("before=%lu\n", (unsigned long)WaitForSingleObject(ev_done, 0));   /* waiter is blocked: timeout */
    SetEvent(ev_go);
    printf("after=%lu\n", (unsigned long)WaitForSingleObject(ev_done, 2000));
    WaitForSingleObject(t2, 2000); DWORD e2; GetExitCodeThread(t2, &e2); printf("t2=%lu\n", (unsigned long)e2);
    HANDLE susp = CreateThread(0, 0, worker, (LPVOID)9, CREATE_SUSPENDED, 0);
    Sleep(10); printf("susp running=%d\n", WaitForSingleObject(susp, 0) == WAIT_TIMEOUT);
    ResumeThread(susp); WaitForSingleObject(susp, 2000); GetExitCodeThread(susp, &e2); printf("susp=%lu\n", (unsigned long)e2);
    return 0;
}
