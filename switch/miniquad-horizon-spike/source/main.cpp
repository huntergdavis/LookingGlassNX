#include <switch.h>
#include <EGL/egl.h>
#include <GLES3/gl3.h>
#include <unistd.h>
#include <switch/services/csrng.h>
#include <algorithm>
#include <cerrno>
#include <cstdint>
#include <limits>
#include <mutex>
#include <cstdio>
#include <cstring>
#include <fcntl.h>
#include <sys/types.h>
#include "sysconf_compat.h"

static_assert(std::numeric_limits<EGLint>::max() == std::numeric_limits<int>::max(),
              "EGLint and viewport argument ranges must match on Switch");
static_assert(std::numeric_limits<GLsizei>::max() == std::numeric_limits<int>::max(),
              "GLsizei and viewport argument ranges must match on Switch");

extern "C" bool miniquad_horizon_start(int width, int height);
extern "C" void miniquad_horizon_frame();
extern "C" void miniquad_horizon_stop();
extern "C" bool miniquad_horizon_should_quit();
extern "C" bool lookingglass_horizon_game_entry();
extern "C" void lg_horizon_diag(const char* event);

// Best-effort stdout + unique SD-card trace. Never truncate a previous run;
// logging errors are intentionally ignored so diagnostics cannot change flow.
static FILE* g_diag = nullptr;
static void diag(const char* event) {
    if (g_diag) { std::fprintf(g_diag, "%s\n", event); std::fflush(g_diag); }
    std::printf("LGNX-DIAG %s\n", event);
}

extern "C" void lg_horizon_diag(const char* event) {
    if (!event) return;
    diag(event);
}

// newlib's libnx sysconf implementation is not linked on this target. Rust
// std needs page size for thread stack alignment and online CPU count for its
// thread pool. Keep these two queries bounded and reject everything else.
static_assert(_SC_PAGESIZE == 8, "pinned newlib _SC_PAGESIZE ABI changed");
static_assert(_SC_NPROCESSORS_ONLN == 10, "pinned newlib _SC_NPROCESSORS_ONLN ABI changed");
extern "C" long sysconf(int name) {
    return lgnx_sysconf_query(name, _SC_PAGESIZE, _SC_NPROCESSORS_ONLN);
}
static void* proc(const char* name) { return reinterpret_cast<void*>(eglGetProcAddress(name)); }
extern "C" void* lg_egl_proc(const char* name) { return proc(name); }
extern "C" void lg_set_gl_viewport(int width, int height) { glViewport(0, 0, width, height); }

// Rust std uses the POSIX getrandom ABI for randomized HashMap seeds. Horizon
// provides the same entropy source through libnx's CSRNG service instead.
static std::mutex g_csrng_mutex;
static bool g_csrng_ready = false;
extern "C" bool lg_csrng_init() {
    std::lock_guard<std::mutex> lock(g_csrng_mutex);
    if (g_csrng_ready) return true;
    g_csrng_ready = R_SUCCEEDED(csrngInitialize());
    return g_csrng_ready;
}
extern "C" void lg_csrng_shutdown() {
    std::lock_guard<std::mutex> lock(g_csrng_mutex);
    if (g_csrng_ready) {
        csrngExit();
        g_csrng_ready = false;
    }
}
extern "C" int lg_csrng_fill(uint8_t* buffer, size_t length) {
    if (length == 0) return 0;
    if (!buffer) return -1;
    if (length > static_cast<size_t>(std::numeric_limits<ssize_t>::max())) return -1;
    std::lock_guard<std::mutex> lock(g_csrng_mutex);
    if (!g_csrng_ready) return -1;
    size_t done = 0;
    while (done < length) {
        const size_t chunk = std::min<size_t>(length - done, 0x1000);
        if (R_FAILED(csrngGetRandomBytes(static_cast<uint8_t*>(buffer) + done, chunk))) {
            return -1;
        }
        done += chunk;
    }
    return 0;
}
extern "C" ssize_t getrandom(void* buffer, size_t length, unsigned int flags) {
    if (flags != 0 || length > static_cast<size_t>(std::numeric_limits<ssize_t>::max())) {
        errno = EINVAL;
        return -1;
    }
    if (length == 0) return 0;
    if (!buffer) { errno = EFAULT; return -1; }
    if (lg_csrng_fill(static_cast<uint8_t*>(buffer), length) != 0) { errno = EIO; return -1; }
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
    // O_EXCL guarantees a rerun never overwrites any existing diagnostic.
    for (unsigned int run = 0; run < 1000 && !g_diag; ++run) {
        char path[96];
        std::snprintf(path, sizeof(path), "sdmc:/switch/alice-nx/startup-diagnostic-%03u.log", run);
        errno = 0;
        int fd = open(path, O_WRONLY | O_CREAT | O_EXCL, 0644);
        if (fd >= 0) {
            g_diag = fdopen(fd, "w");
            if (!g_diag) close(fd);
            break;
        }
        if (errno != EEXIST) break;
    }
    diag("main:entered");
    if (!lg_csrng_init()) { diag("exit:csrng-init-failed:10"); return 10; }
    diag("csrng:ready");
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    padInitializeDefault(&g_pad);
    NWindow* window = nwindowGetDefault();
    EGLDisplay display = eglGetDisplay(EGL_DEFAULT_DISPLAY);
    if (display == EGL_NO_DISPLAY || !eglInitialize(display, nullptr, nullptr)) {
        char event[64]; std::snprintf(event, sizeof(event), "exit:egl-initialize-failed:error=0x%04x:1", eglGetError()); diag(event);
        lg_csrng_shutdown(); return 1;
    }
    diag("egl:initialized");
    EGLSurface surface = EGL_NO_SURFACE;
    EGLContext context = EGL_NO_CONTEXT;
    auto cleanup = [&]() {
        eglMakeCurrent(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
        if (context != EGL_NO_CONTEXT) eglDestroyContext(display, context);
        if (surface != EGL_NO_SURFACE) eglDestroySurface(display, surface);
        eglTerminate(display);
        lg_csrng_shutdown();
    };
    if (!eglBindAPI(EGL_OPENGL_ES_API)) { char e[64]; std::snprintf(e,sizeof(e),"exit:egl-bind-api:error=0x%04x:2",eglGetError()); diag(e); cleanup(); return 2; }
    const EGLint attrs[] = {EGL_RED_SIZE,8,EGL_GREEN_SIZE,8,EGL_BLUE_SIZE,8,EGL_ALPHA_SIZE,8,EGL_DEPTH_SIZE,24,EGL_SURFACE_TYPE,EGL_WINDOW_BIT,EGL_RENDERABLE_TYPE,EGL_OPENGL_ES3_BIT,EGL_NONE};
    EGLConfig config{}; EGLint count=0;
    if (!eglChooseConfig(display, attrs, &config, 1, &count) || count != 1) { char e[64]; std::snprintf(e,sizeof(e),"exit:egl-choose-config:error=0x%04x:count=%d:3",eglGetError(),count); diag(e); cleanup(); return 3; }
    surface=eglCreateWindowSurface(display,config,(EGLNativeWindowType)window,nullptr);
    if (surface==EGL_NO_SURFACE) { char e[64]; std::snprintf(e,sizeof(e),"exit:egl-create-surface:error=0x%04x:4",eglGetError()); diag(e); cleanup(); return 4; }
    diag("egl:surface-created");
    const EGLint context_attrs[]={EGL_CONTEXT_CLIENT_VERSION,3,EGL_NONE};
    context=eglCreateContext(display,config,EGL_NO_CONTEXT,context_attrs);
    if (context==EGL_NO_CONTEXT || !eglMakeCurrent(display,surface,surface,context)) { char e[64]; std::snprintf(e,sizeof(e),"exit:egl-make-current:error=0x%04x:5",eglGetError()); diag(e); cleanup(); return 5; }
    diag("egl:context-current");
    glClearColor(0.f, 0.f, 0.f, 1.f);
    glClear(GL_COLOR_BUFFER_BIT);
    const GLenum clear_error=glGetError();
    if (clear_error != GL_NO_ERROR) {
        char e[80]; std::snprintf(e,sizeof(e),"exit:bootstrap-clear-gl-error:0x%04x:14",clear_error);
        diag(e); cleanup(); return 14;
    }
    diag("gl:bootstrap-clear-no-error");
    if (!eglSwapBuffers(display, surface)) { char e[64]; std::snprintf(e,sizeof(e),"exit:egl-bootstrap-swap-failed:error=0x%04x:9",eglGetError()); diag(e); cleanup(); return 9; }
    diag("egl:bootstrap-swap-succeeded");
    const GLubyte* version = glGetString(GL_VERSION);
    if (!version) { diag("exit:gl-version-null-after-bootstrap-swap:12"); cleanup(); return 12; }
    size_t version_length = 0;
    while (version_length < 256 && version[version_length] != '\0') {
        const unsigned char ch = version[version_length];
        if (ch < 0x20 || ch > 0x7e) { diag("exit:invalid-gl-version-string:13"); cleanup(); return 13; }
        ++version_length;
    }
    if (version_length == 0 || version_length == 256) { diag("exit:invalid-gl-version-string:13"); cleanup(); return 13; }
    if (std::strncmp(reinterpret_cast<const char*>(version), "OpenGL ES 3.", 12) != 0) {
        diag("exit:unsupported-gl-version:13"); cleanup(); return 13;
    }
    { char e[128]; std::snprintf(e, sizeof(e), "gl:GL_VERSION:%.*s", static_cast<int>(version_length), reinterpret_cast<const char*>(version)); diag(e); }
    GLint max_viewport[2]={0,0};
    glGetIntegerv(GL_MAX_VIEWPORT_DIMS,max_viewport);
    const GLenum viewport_error=glGetError();
    if (viewport_error != GL_NO_ERROR || max_viewport[0] <= 0 || max_viewport[1] <= 0) {
        char e[112]; std::snprintf(e,sizeof(e),"exit:viewport-limits-invalid:error=0x%04x:max=%dx%d:15",
                                   viewport_error,max_viewport[0],max_viewport[1]);
        diag(e); cleanup(); return 15;
    }
    { char e[80]; std::snprintf(e,sizeof(e),"gl:max-viewport:%dx%d",max_viewport[0],max_viewport[1]); diag(e); }
    EGLint width=0,height=0;
    const EGLBoolean width_query_ok=eglQuerySurface(display,surface,EGL_WIDTH,&width);
    const EGLint width_query_error=width_query_ok ? EGL_SUCCESS : eglGetError();
    const EGLBoolean height_query_ok=eglQuerySurface(display,surface,EGL_HEIGHT,&height);
    const EGLint height_query_error=height_query_ok ? EGL_SUCCESS : eglGetError();
    { char e[96]; std::snprintf(e,sizeof(e),"egl:query-width:ok=%u:value=%d:error=0x%04x",static_cast<unsigned>(width_query_ok),width,width_query_error); diag(e); }
    { char e[96]; std::snprintf(e,sizeof(e),"egl:query-height:ok=%u:value=%d:error=0x%04x",static_cast<unsigned>(height_query_ok),height,height_query_error); diag(e); }
    const bool egl_size_usable = width_query_ok && height_query_ok && width > 0 && height > 0 &&
        width <= std::numeric_limits<int>::max() && height <= std::numeric_limits<int>::max();
    if (!egl_size_usable) {
        u32 native_width=0,native_height=0;
        const Result native_size_result=nwindowGetDimensions(window,&native_width,&native_height);
        if (R_FAILED(native_size_result) || native_width == 0 || native_height == 0 ||
            native_width > static_cast<u32>(std::numeric_limits<int>::max()) ||
            native_height > static_cast<u32>(std::numeric_limits<int>::max())) {
            char e[128];
            std::snprintf(e,sizeof(e),"exit:native-window-size-invalid:result=0x%08x:size=%ux%u",native_size_result,native_width,native_height);
            diag(e); cleanup(); return 6;
        }
        width=static_cast<EGLint>(native_width);
        height=static_cast<EGLint>(native_height);
        { char e[96]; std::snprintf(e,sizeof(e),"window:viewport-source=nwindowGetDimensions:size=%dx%d",width,height); diag(e); }
    } else {
        diag("window:viewport-source=eglQuerySurface");
    }
    if (width > max_viewport[0] || height > max_viewport[1]) {
        char e[112]; std::snprintf(e,sizeof(e),"exit:viewport-size-exceeds-limit:size=%dx%d:max=%dx%d:16",
                                   width,height,max_viewport[0],max_viewport[1]);
        diag(e); cleanup(); return 16;
    }
    if (!proc("glGenVertexArrays")) { diag("exit:missing-gl-proc:glGenVertexArrays:7"); cleanup(); return 7; }
    if (!proc("glBindVertexArray")) { diag("exit:missing-gl-proc:glBindVertexArray:7"); cleanup(); return 7; }
    if (!proc("glDeleteVertexArrays")) { diag("exit:missing-gl-proc:glDeleteVertexArrays:7"); cleanup(); return 7; }
    if (!miniquad_horizon_start(static_cast<int>(width),static_cast<int>(height))) { diag("exit:miniquad-start-or-gl-proc:8"); cleanup(); return 8; }
    diag("miniquad:native-started");
    // Runner start installs the handler only after native display and GL init.
    if (!lookingglass_horizon_game_entry()) {
        diag("exit:rust-entry-failed:11");
        miniquad_horizon_stop();
        cleanup();
        return 11;
    }
    diag("rust:entry-accepted");
    int result=0;
    bool first_frame = true;
    bool first_swap = true;
    while (appletMainLoop()) {
        padUpdate(&g_pad);
        miniquad_horizon_frame();
        if (first_frame) { diag("frame:first-draw-returned"); first_frame = false; }
        if (miniquad_horizon_should_quit()) break;
        if (!eglSwapBuffers(display,surface)) { char e[64]; std::snprintf(e,sizeof(e),"exit:egl-swap-failed:error=0x%04x:9",eglGetError()); diag(e); result=9; break; }
        if (first_swap) { diag("frame:first-swap-succeeded"); first_swap = false; }
    }
    miniquad_horizon_stop();
    diag("exit:host-loop-finished");
    cleanup();
    if (g_diag) { std::fclose(g_diag); g_diag = nullptr; }
    return result;
}
