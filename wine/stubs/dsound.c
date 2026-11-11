/* Plonk dsound.dll stub: DirectSound reports "no driver" so games continue silently.
 * (Real output via WebAudio is a later project.) Exports are stdcall; names are undecorated via dsound.def. */
#include <windows.h>

#define DSERR_NODRIVER ((HRESULT)0x88780078L)

HRESULT WINAPI DirectSoundCreate(LPVOID dev, LPVOID *ds, LPVOID outer) { if (ds) *ds = 0; return DSERR_NODRIVER; }
HRESULT WINAPI DirectSoundCreate8(LPVOID dev, LPVOID *ds, LPVOID outer) { if (ds) *ds = 0; return DSERR_NODRIVER; }
HRESULT WINAPI DirectSoundCaptureCreate(LPVOID dev, LPVOID *ds, LPVOID outer) { if (ds) *ds = 0; return DSERR_NODRIVER; }
HRESULT WINAPI DirectSoundCaptureCreate8(LPVOID dev, LPVOID *ds, LPVOID outer) { if (ds) *ds = 0; return DSERR_NODRIVER; }
HRESULT WINAPI DirectSoundEnumerateA(LPVOID cb, LPVOID ctx) { return S_OK; }
HRESULT WINAPI DirectSoundEnumerateW(LPVOID cb, LPVOID ctx) { return S_OK; }
HRESULT WINAPI DirectSoundCaptureEnumerateA(LPVOID cb, LPVOID ctx) { return S_OK; }
HRESULT WINAPI DirectSoundCaptureEnumerateW(LPVOID cb, LPVOID ctx) { return S_OK; }
HRESULT WINAPI DirectSoundFullDuplexCreate(LPVOID a, LPVOID b, LPVOID c, LPVOID d, LPVOID e, LPVOID f, LPVOID g, LPVOID h, LPVOID i, LPVOID j) { return DSERR_NODRIVER; }
HRESULT WINAPI GetDeviceID(LPVOID in, LPVOID out) { return DSERR_NODRIVER; }
HRESULT WINAPI DllGetClassObject(REFCLSID c, REFIID i, LPVOID *o) { if (o) *o = 0; return CLASS_E_CLASSNOTAVAILABLE; }
HRESULT WINAPI DllCanUnloadNow(void) { return S_FALSE; }
BOOL WINAPI DllMain(HINSTANCE h, DWORD why, LPVOID r) { return TRUE; }
