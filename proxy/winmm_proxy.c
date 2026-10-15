// Proxy winmm.dll: repassa tudo para winmm_real.dll (copia do SysWOW64\winmm.dll)
// e registra o trafego MIDI (short + SysEx) em ampero_midi_<timestamp>.log.
#include <windows.h>
#include <mmsystem.h>
#include <stdio.h>

static HMODULE g_real, g_self;
static FILE *g_log;
static CRITICAL_SECTION g_cs;
static LARGE_INTEGER g_freq, g_t0;

typedef MMRESULT (WINAPI *pfnMidiOutOpen)(LPHMIDIOUT, UINT, DWORD_PTR, DWORD_PTR, DWORD);
typedef MMRESULT (WINAPI *pfnMidiOutShortMsg)(HMIDIOUT, DWORD);
typedef MMRESULT (WINAPI *pfnMidiOutLongMsg)(HMIDIOUT, LPMIDIHDR, UINT);
typedef MMRESULT (WINAPI *pfnMidiInOpen)(LPHMIDIIN, UINT, DWORD_PTR, DWORD_PTR, DWORD);
typedef MMRESULT (WINAPI *pfnMidiInGetDevCapsW)(UINT_PTR, LPMIDIINCAPSW, UINT);
typedef MMRESULT (WINAPI *pfnMidiOutGetDevCapsW)(UINT_PTR, LPMIDIOUTCAPSW, UINT);

static pfnMidiOutOpen r_midiOutOpen;
static pfnMidiOutShortMsg r_midiOutShortMsg;
static pfnMidiOutLongMsg r_midiOutLongMsg;
static pfnMidiInOpen r_midiInOpen;
static pfnMidiInGetDevCapsW r_midiInGetDevCapsW;
static pfnMidiOutGetDevCapsW r_midiOutGetDevCapsW;

static void ensure_init(void)
{
    if (g_real) return;
    EnterCriticalSection(&g_cs);
    if (!g_real) {
        char path[MAX_PATH], *slash;
        GetModuleFileNameA(g_self, path, MAX_PATH);
        slash = strrchr(path, '\\');
        strcpy(slash + 1, "winmm_real.dll");
        HMODULE h = LoadLibraryA(path);
        r_midiOutOpen = (pfnMidiOutOpen)GetProcAddress(h, "midiOutOpen");
        r_midiOutShortMsg = (pfnMidiOutShortMsg)GetProcAddress(h, "midiOutShortMsg");
        r_midiOutLongMsg = (pfnMidiOutLongMsg)GetProcAddress(h, "midiOutLongMsg");
        r_midiInOpen = (pfnMidiInOpen)GetProcAddress(h, "midiInOpen");
        r_midiInGetDevCapsW = (pfnMidiInGetDevCapsW)GetProcAddress(h, "midiInGetDevCapsW");
        r_midiOutGetDevCapsW = (pfnMidiOutGetDevCapsW)GetProcAddress(h, "midiOutGetDevCapsW");

        char logdir[MAX_PATH], logpath[MAX_PATH];
        if (!GetEnvironmentVariableA("AMPERO_LOG_DIR", logdir, MAX_PATH)) {
            strcpy(logdir, path);
            *strrchr(logdir, '\\') = 0;
        }
        SYSTEMTIME st;
        GetLocalTime(&st);
        snprintf(logpath, MAX_PATH, "%s\\ampero_midi_%04d%02d%02d_%02d%02d%02d.log", logdir,
                 st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond);
        g_log = fopen(logpath, "w");
        QueryPerformanceFrequency(&g_freq);
        QueryPerformanceCounter(&g_t0);
        g_real = h;
    }
    LeaveCriticalSection(&g_cs);
}

static double now_ms(void)
{
    LARGE_INTEGER t;
    QueryPerformanceCounter(&t);
    return (double)(t.QuadPart - g_t0.QuadPart) * 1000.0 / (double)g_freq.QuadPart;
}

static void log_bytes(const char *tag, const void *handle, const BYTE *p, DWORD n)
{
    if (!g_log) return;
    EnterCriticalSection(&g_cs);
    fprintf(g_log, "%10.3f %s %p %4lu:", now_ms(), tag, handle, (unsigned long)n);
    for (DWORD i = 0; i < n; i++) fprintf(g_log, " %02X", p[i]);
    fputc('\n', g_log);
    fflush(g_log);
    LeaveCriticalSection(&g_cs);
}

static void log_text(const char *fmt, ...)
{
    if (!g_log) return;
    va_list ap;
    EnterCriticalSection(&g_cs);
    fprintf(g_log, "%10.3f # ", now_ms());
    va_start(ap, fmt);
    vfprintf(g_log, fmt, ap);
    va_end(ap);
    fputc('\n', g_log);
    fflush(g_log);
    LeaveCriticalSection(&g_cs);
}

static void short_to_bytes(DWORD msg, BYTE out[3], DWORD *len)
{
    out[0] = msg & 0xFF; out[1] = (msg >> 8) & 0xFF; out[2] = (msg >> 16) & 0xFF;
    BYTE s = out[0] & 0xF0;
    *len = (s == 0xC0 || s == 0xD0) ? 2 : 3;
}

// ---- saida ----

MMRESULT WINAPI hk_midiOutOpen(LPHMIDIOUT ph, UINT dev, DWORD_PTR cb, DWORD_PTR inst, DWORD flags)
{
    ensure_init();
    MIDIOUTCAPSW caps = {0};
    r_midiOutGetDevCapsW(dev, &caps, sizeof caps);
    MMRESULT r = r_midiOutOpen(ph, dev, cb, inst, flags);
    log_text("midiOutOpen dev=%u name=\"%ls\" flags=%08lX -> %u handle=%p", dev, caps.szPname,
             (unsigned long)flags, r, ph ? (void *)*ph : NULL);
    return r;
}

MMRESULT WINAPI hk_midiOutShortMsg(HMIDIOUT h, DWORD msg)
{
    ensure_init();
    BYTE b[3]; DWORD n;
    short_to_bytes(msg, b, &n);
    log_bytes("OUT", h, b, n);
    return r_midiOutShortMsg(h, msg);
}

MMRESULT WINAPI hk_midiOutLongMsg(HMIDIOUT h, LPMIDIHDR hdr, UINT cb)
{
    ensure_init();
    if (hdr && hdr->lpData) log_bytes("OUT", h, (BYTE *)hdr->lpData, hdr->dwBufferLength);
    return r_midiOutLongMsg(h, hdr, cb);
}

// ---- entrada: troca o callback para espiar MIM_DATA / MIM_LONGDATA ----

typedef struct { DWORD_PTR cb, inst; } InCtx;

static void CALLBACK in_trampoline(HMIDIIN h, UINT msg, DWORD_PTR inst, DWORD_PTR p1, DWORD_PTR p2)
{
    InCtx *ctx = (InCtx *)inst;
    if (msg == MIM_DATA) {
        BYTE b[3]; DWORD n;
        short_to_bytes((DWORD)p1, b, &n);
        log_bytes("IN ", h, b, n);
    } else if (msg == MIM_LONGDATA || msg == MIM_LONGERROR) {
        MIDIHDR *hdr = (MIDIHDR *)p1;
        if (hdr && hdr->dwBytesRecorded)
            log_bytes(msg == MIM_LONGDATA ? "IN " : "INE", h, (BYTE *)hdr->lpData, hdr->dwBytesRecorded);
    } else if (msg == MIM_OPEN || msg == MIM_CLOSE) {
        log_text("midiIn %s handle=%p", msg == MIM_OPEN ? "OPEN" : "CLOSE", h);
    }
    if (ctx->cb)
        ((void (CALLBACK *)(HMIDIIN, UINT, DWORD_PTR, DWORD_PTR, DWORD_PTR))ctx->cb)(h, msg, ctx->inst, p1, p2);
}

MMRESULT WINAPI hk_midiInOpen(LPHMIDIIN ph, UINT dev, DWORD_PTR cb, DWORD_PTR inst, DWORD flags)
{
    ensure_init();
    MIDIINCAPSW caps = {0};
    r_midiInGetDevCapsW(dev, &caps, sizeof caps);
    MMRESULT r;
    if ((flags & CALLBACK_TYPEMASK) == CALLBACK_FUNCTION) {
        InCtx *ctx = (InCtx *)HeapAlloc(GetProcessHeap(), 0, sizeof *ctx); // vaza por design: vive enquanto a porta existir
        ctx->cb = cb; ctx->inst = inst;
        r = r_midiInOpen(ph, dev, (DWORD_PTR)in_trampoline, (DWORD_PTR)ctx, flags);
    } else {
        r = r_midiInOpen(ph, dev, cb, inst, flags);
        log_text("AVISO: midiInOpen sem CALLBACK_FUNCTION (flags=%08lX), entrada nao sera logada", (unsigned long)flags);
    }
    log_text("midiInOpen dev=%u name=\"%ls\" flags=%08lX -> %u handle=%p", dev, caps.szPname,
             (unsigned long)flags, r, ph ? (void *)*ph : NULL);
    return r;
}

BOOL WINAPI DllMain(HINSTANCE inst, DWORD reason, LPVOID reserved)
{
    if (reason == DLL_PROCESS_ATTACH) {
        g_self = inst;
        InitializeCriticalSection(&g_cs);
        DisableThreadLibraryCalls(inst);
    } else if (reason == DLL_PROCESS_DETACH && g_log) {
        fclose(g_log);
    }
    return TRUE;
}
