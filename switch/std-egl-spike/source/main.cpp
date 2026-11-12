#include <switch.h>
#include <EGL/egl.h>
#include <switch/services/csrng.h>
#include <cstddef>
#include <cstdint>
#include <cerrno>
#include <sys/types.h>

extern "C" bool lookingglass_miniquad_init();
extern "C" bool lookingglass_miniquad_frame(std::uint64_t buttons);
extern "C" void lookingglass_miniquad_exit();
extern "C" void* lookingglass_get_proc_address(const char* name);
extern "C" int lookingglass_fill_random(void* dest, std::size_t len);

void* lookingglass_get_proc_address(const char* name) {
    return reinterpret_cast<void*>(eglGetProcAddress(name));
}

int lookingglass_fill_random(void* dest, std::size_t len) {
    return R_SUCCEEDED(csrngGetRandomBytes(dest, len)) ? 0 : -1;
}

struct Runtime {
    EGLDisplay display{EGL_NO_DISPLAY};
    EGLSurface surface{EGL_NO_SURFACE};
    EGLContext context{EGL_NO_CONTEXT};
    bool egl_initialized{false};
    bool csrng_ready{false};
    bool miniquad_ready{false};

    ~Runtime() {
        if (miniquad_ready) lookingglass_miniquad_exit();
        if (display != EGL_NO_DISPLAY && egl_initialized) {
            eglMakeCurrent(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
            if (context != EGL_NO_CONTEXT) eglDestroyContext(display, context);
            if (surface != EGL_NO_SURFACE) eglDestroySurface(display, surface);
            eglTerminate(display);
        }
        if (csrng_ready) csrngExit();
    }
};

extern "C" ssize_t getrandom(void* dest, std::size_t len, unsigned int flags) {
    if (flags != 0) {
        errno = EINVAL;
        return -1;
    }
    if (R_FAILED(csrngGetRandomBytes(dest, len))) {
        errno = EIO;
        return -1;
    }
    return static_cast<ssize_t>(len);
}

int main(int, char**) {
    Runtime runtime;
    if (R_FAILED(csrngInitialize())) return 6;
    runtime.csrng_ready = true;
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    PadState pad{};
    padInitializeDefault(&pad);

    runtime.display = eglGetDisplay(EGL_DEFAULT_DISPLAY);
    if (runtime.display == EGL_NO_DISPLAY || !eglInitialize(runtime.display, nullptr, nullptr)) return 1;
    runtime.egl_initialized = true;
    if (!eglBindAPI(EGL_OPENGL_ES_API)) return 2;

    const EGLint config_attrs[] = {
        EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8,
        EGL_ALPHA_SIZE, 8, EGL_DEPTH_SIZE, 24, EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
        EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE
    };
    EGLConfig config{};
    EGLint count = 0;
    if (!eglChooseConfig(runtime.display, config_attrs, &config, 1, &count) || count < 1) return 3;
    runtime.surface = eglCreateWindowSurface(
        runtime.display, config, (EGLNativeWindowType)nwindowGetDefault(), nullptr);
    if (runtime.surface == EGL_NO_SURFACE) return 4;
    const EGLint context_attrs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
    runtime.context = eglCreateContext(runtime.display, config, EGL_NO_CONTEXT, context_attrs);
    if (runtime.context == EGL_NO_CONTEXT || !eglMakeCurrent(runtime.display, runtime.surface, runtime.surface, runtime.context)) return 5;
    if (!lookingglass_miniquad_init()) return 8;
    runtime.miniquad_ready = true;

    int result = 0;
    while (appletMainLoop()) {
        padUpdate(&pad);
        if (padGetButtonsDown(&pad) & HidNpadButton_Plus) break;
        const bool a_held = (padGetButtons(&pad) & HidNpadButton_A) != 0;
        if (!lookingglass_miniquad_frame(a_held ? 1 : 0)) { result = 9; break; }
        if (!eglSwapBuffers(runtime.display, runtime.surface)) { result = 7; break; }
    }
    return result;
}
