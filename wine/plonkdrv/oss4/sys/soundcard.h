/* Minimal OSSv4 additions on top of glibc's OSSv3 header, so wineoss.drv builds. */
#ifndef PLONK_OSS4_SOUNDCARD_H
#define PLONK_OSS4_SOUNDCARD_H
#include_next <sys/soundcard.h>

typedef struct oss_sysinfo {
    char product[32];
    char version[32];
    int versionnum;
    char options[128];
    int numaudios;
    int openedaudio[8];
    int numsynths;
    int nummidis;
    int numtimers;
    int nummixers;
    int openedmidi[8];
    int numcards;
    int numaudioengines;
    char license[16];
    char revision_info[256];
    int filler[172];
} oss_sysinfo;

typedef struct oss_audioinfo {
    int dev;
    char name[64];
    int busy;
    int pid;
    int caps;
    int iformats;
    int oformats;
    int magic;
    char cmd[64];
    int card_number;
    int port_number;
    int mixer_dev;
    int legacy_device;
    int enabled;
    int flags;
    int min_rate;
    int max_rate;
    int min_channels;
    int max_channels;
    int binding;
    int rate_source;
    char handle[32];
    unsigned int nrates;
    unsigned int rates[20];
    char song_name[64];
    char label[16];
    int latency;
    char devnode[32];
    int next_play_engine;
    int next_rec_engine;
    int filler[184];
} oss_audioinfo;

#define SNDCTL_SYSINFO     _IOR('X', 1, oss_sysinfo)
#define SNDCTL_AUDIOINFO   _IOWR('X', 7, oss_audioinfo)
#define SNDCTL_ENGINEINFO  _IOWR('X', 12, oss_audioinfo)
#define OSS_DEVNODE_SIZE 32
#ifndef AFMT_S24_LE
#define AFMT_S24_LE 0x00000800
#endif
#ifndef AFMT_S32_LE
#define AFMT_S32_LE 0x00001000
#endif
#ifndef AFMT_FLOAT
#define AFMT_FLOAT 0x00004000
#endif
#ifndef PCM_CAP_INPUT
#define PCM_CAP_INPUT  0x00010000
#define PCM_CAP_OUTPUT 0x00020000
#endif
#endif
