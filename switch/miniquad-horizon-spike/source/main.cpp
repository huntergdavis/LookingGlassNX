#include <switch.h>
#include <EGL/egl.h>
#include <GLES3/gl3.h>
#include <switch/services/csrng.h>
#include <cerrno>
#include <cstdint>
#include <sys/types.h>

extern "C" bool miniquad_horizon_start(int width, int height);
extern "C" void miniquad_horizon_frame();
extern "C" void miniquad_horizon_stop();
extern "C" bool miniquad_horizon_should_quit();
extern "C" void lookingglass_horizon_entry();
static void* proc(const char* name) { return reinterpret_cast<void*>(eglGetProcAddress(name)); }
extern "C" void* lg_egl_proc(const char* name) { return proc(name); }
extern "C" void lg_set_gl_viewport(int width, int height) { glViewport(0, 0, width, height); }

// Rust std uses the POSIX getrandom ABI for randomized HashMap seeds. Horizon
// provides the same entropy source through libnx's CSRNG service instead.
extern "C" ssize_t getrandom(void* buffer, size_t length, unsigned int flags) {
    if (flags != 0 || (!buffer && length != 0)) { errno = EINVAL; return -1; }
    if (R_FAILED(csrngGetRandomBytes(buffer, length))) { errno = EIO; return -1; }
    return static_cast<ssize_t>(length);
}

static PadState g_pad{};
extern "C" bool lg_hid_connected(uint32_t index) {
    return index == 0 && padIsConnected(&g_pad);
}
extern "C" bool lg_hid_buttons(uint32_t index, uint64_t* buttons) {
    if (index != 0 || !buttons || !padIsConnected(&g_pad)) return false;
    *buttons = padGetButtons(&g_pad); return true;
}
extern "C" bool lg_hid_axes(uint32_t index, int16_t* lx, int16_t* ly, int16_t* rx, int16_t* ry) {
    if (index != 0 || !lx || !ly || !rx || !ry) return false;
    HidAnalogStickState l=padGetStickPos(&g_pad, 0), r=padGetStickPos(&g_pad, 1);
    auto clamp16=[](int32_t v) { return static_cast<int16_t>(v < -32767 ? -32767 : (v > 32767 ? 32767 : v)); };
    *lx=clamp16(l.x); *ly=clamp16(l.y); *rx=clamp16(r.x); *ry=clamp16(r.y); return true;
}

int main(int, char**) {
    if (R_FAILED(csrngInitialize())) return 10;
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    padInitializeDefault(&g_pad);
    EGLDisplay display = eglGetDisplay(EGL_DEFAULT_DISPLAY);
    if (display == EGL_NO_DISPLAY || !eglInitialize(display, nullptr, nullptr)) { csrngExit(); return 1; }
    EGLSurface surface = EGL_NO_SURFACE;
    EGLContext context = EGL_NO_CONTEXT;
    auto cleanup = [&]() {
        eglMakeCurrent(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
        if (context != EGL_NO_CONTEXT) eglDestroyContext(display, context);
        if (surface != EGL_NO_SURFACE) eglDestroySurface(display, surface);
        eglTerminate(display);
        csrngExit();
    };
    if (!eglBindAPI(EGL_OPENGL_ES_API)) { cleanup(); return 2; }
    const EGLint attrs[] = {EGL_RED_SIZE,8,EGL_GREEN_SIZE,8,EGL_BLUE_SIZE,8,EGL_ALPHA_SIZE,8,EGL_DEPTH_SIZE,24,EGL_SURFACE_TYPE,EGL_WINDOW_BIT,EGL_RENDERABLE_TYPE,EGL_OPENGL_ES3_BIT,EGL_NONE};
    EGLConfig config{}; EGLint count=0;
    if (!eglChooseConfig(display, attrs, &config, 1, &count) || count != 1) { cleanup(); return 3; }
    surface=eglCreateWindowSurface(display,config,(EGLNativeWindowType)nwindowGetDefault(),nullptr);
    if (surface==EGL_NO_SURFACE) { cleanup(); return 4; }
    const EGLint context_attrs[]={EGL_CONTEXT_CLIENT_VERSION,3,EGL_NONE};
    context=eglCreateContext(display,config,EGL_NO_CONTEXT,context_attrs);
    if (context==EGL_NO_CONTEXT || !eglMakeCurrent(display,surface,surface,context)) { cleanup(); return 5; }
    EGLint width=0,height=0;
    if (!eglQuerySurface(display,surface,EGL_WIDTH,&width) || !eglQuerySurface(display,surface,EGL_HEIGHT,&height) || width<=0 || height<=0) { cleanup(); return 6; }
    if (!proc("glGenVertexArrays") || !proc("glBindVertexArray") || !proc("glDeleteVertexArrays")) { cleanup(); return 7; }
    if (!miniquad_horizon_start(width,height)) { cleanup(); return 8; }
    // Runner start installs the handler only after native display and GL init.
    lookingglass_horizon_entry();
    int result=0;
    while (appletMainLoop()) {
        padUpdate(&g_pad);
        const uint64_t down=padGetButtonsDown(&g_pad);
        if (down & HidNpadButton_Plus) break;
        miniquad_horizon_frame();
        if (miniquad_horizon_should_quit()) break;
        if (!eglSwapBuffers(display,surface)) { (void)eglGetError(); result=9; break; }
    }
    miniquad_horizon_stop();
    cleanup();
    return result;
}
