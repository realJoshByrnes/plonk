/* waveOut sine test: plays 1 s of 440 Hz and reports what happened. */
#include <windows.h>
#include <mmsystem.h>
#include <stdio.h>
#include <math.h>
static short buf[44100 * 2];
int main(void) {
    WAVEFORMATEX f = { WAVE_FORMAT_PCM, 2, 44100, 44100 * 4, 4, 16, 0 };
    HWAVEOUT h; WAVEHDR hd = { 0 };
    printf("devs=%u\n", waveOutGetNumDevs());
    MMRESULT r = waveOutOpen(&h, WAVE_MAPPER, &f, 0, 0, CALLBACK_NULL);
    printf("open=%u\n", r);
    if (r) return 1;
    for (int i = 0; i < 44100; i++) buf[2 * i] = buf[2 * i + 1] = (short)(12000 * sin(i * 6.2831853 * 440 / 44100));
    hd.lpData = (LPSTR)buf; hd.dwBufferLength = sizeof buf;
    waveOutPrepareHeader(h, &hd, sizeof hd);
    r = waveOutWrite(h, &hd, sizeof hd);
    printf("write=%u\n", r);
    for (int i = 0; i < 300 && !(hd.dwFlags & WHDR_DONE); i++) Sleep(10);
    printf("done=%d\n", !!(hd.dwFlags & WHDR_DONE));
    waveOutClose(h);
    return 0;
}
