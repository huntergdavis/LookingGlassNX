#include <switch.h>
#include <EGL/egl.h>
#include <GLES2/gl2.h>
#include <switch/services/csrng.h>
#include <cstdint>
#include <cerrno>
#include <sys/types.h>

extern "C" std::uint32_t lookingglass_std_smoke();
extern "C" void lookingglass_miniquad_smoke();
extern "C" void* lookingglass_get_proc_address(const char* name);
extern "C" int lookingglass_fill_random(void* dest, std::size_t len);

void* lookingglass_get_proc_address(const char* name) {
    return reinterpret_cast<void*>(eglGetProcAddress(name));
}

int lookingglass_fill_random(void* dest, std::size_t len) {
    return R_SUCCEEDED(csrngGetRandomBytes(dest, len)) ? 0 : -1;
}

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
    if (R_FAILED(csrngInitialize())) return 6;
    const std::uint32_t std_probe_value = lookingglass_std_smoke();
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    PadState pad{};
    padInitializeDefault(&pad);

    EGLDisplay display = eglGetDisplay(EGL_DEFAULT_DISPLAY);
    if (display == EGL_NO_DISPLAY || !eglInitialize(display, nullptr, nullptr)) return 1;
    if (!eglBindAPI(EGL_OPENGL_ES_API)) return 2;

    const EGLint config_attrs[] = {
        EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8,
        EGL_ALPHA_SIZE, 8, EGL_DEPTH_SIZE, 24, EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
        EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE
    };
    EGLConfig config{};
    EGLint count = 0;
    if (!eglChooseConfig(display, config_attrs, &config, 1, &count) || count < 1) return 3;
    EGLSurface surface = eglCreateWindowSurface(
        display, config, (EGLNativeWindowType)nwindowGetDefault(), nullptr);
    if (surface == EGL_NO_SURFACE) return 4;
    const EGLint context_attrs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
    EGLContext context = eglCreateContext(display, config, EGL_NO_CONTEXT, context_attrs);
    if (context == EGL_NO_CONTEXT || !eglMakeCurrent(display, surface, surface, context)) return 5;

    lookingglass_miniquad_smoke();

    while (appletMainLoop()) {
        padUpdate(&pad);
        if (padGetButtonsDown(&pad) & (HidNpadButton_Plus | HidNpadButton_B)) break;
        glViewport(0, 0, 1280, 720);
        glClearColor(0.08f, 0.18f, 0.38f + (std_probe_value & 1u) * 0.1f, 1.0f);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        if (!eglSwapBuffers(display, surface)) break;
    }

    eglMakeCurrent(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
    eglDestroyContext(display, context);
    eglDestroySurface(display, surface);
    eglTerminate(display);
    csrngExit();
    return 0;
}
