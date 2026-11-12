#include <switch.h>
#include <EGL/egl.h>
#include <GLES2/gl2.h>
#include <cstddef>
#include <cstdint>

extern "C" bool lookingglass_boot_color(std::uint32_t buttons, float* rgba);

struct Runtime {
    EGLDisplay display{EGL_NO_DISPLAY};
    EGLSurface surface{EGL_NO_SURFACE};
    EGLContext context{EGL_NO_CONTEXT};
    bool egl_initialized{false};

    ~Runtime() {
        if (display != EGL_NO_DISPLAY) {
            if (egl_initialized) eglMakeCurrent(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
            if (context != EGL_NO_CONTEXT) eglDestroyContext(display, context);
            if (surface != EGL_NO_SURFACE) eglDestroySurface(display, surface);
            eglTerminate(display);
        }
    }
};

int main(int, char**) {
    Runtime runtime;
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
    EGLint width = 0;
    EGLint height = 0;
    if (!eglQuerySurface(runtime.display, runtime.surface, EGL_WIDTH, &width) ||
        !eglQuerySurface(runtime.display, runtime.surface, EGL_HEIGHT, &height)) {
        (void)eglGetError();
        return 6;
    }
    if (width <= 0 || height <= 0) return 6;
    glViewport(0, 0, width, height);
    if (glGetError() != GL_NO_ERROR) return 8;
    int result = 0;
    while (appletMainLoop()) {
        padUpdate(&pad);
        if (padGetButtonsDown(&pad) & HidNpadButton_Plus) break;
        const bool a_held = (padGetButtons(&pad) & HidNpadButton_A) != 0;
        float color[4];
        if (!lookingglass_boot_color(a_held ? 1 : 0, color)) { result = 9; break; }
        glClearColor(color[0], color[1], color[2], color[3]);
        glClear(GL_COLOR_BUFFER_BIT);
        if (glGetError() != GL_NO_ERROR) { result = 10; break; }
        if (!eglSwapBuffers(runtime.display, runtime.surface)) {
            (void)eglGetError();
            result = 7;
            break;
        }
    }
    return result;
}
