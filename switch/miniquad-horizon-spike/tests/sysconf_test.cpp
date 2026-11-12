#include "../include/sysconf_compat.h"
#include <cassert>

int main() {
    errno = 0;
    assert(lgnx_sysconf_query(8, 8, 10) == 4096);
    assert(errno == 0);
    assert(lgnx_sysconf_query(10, 8, 10) == 1);
    assert(errno == 0);
    assert(lgnx_sysconf_query(999, 8, 10) == -1);
    assert(errno == EINVAL);
    // This is the value consumed unchecked by Rust std's page-size fallback.
    assert(lgnx_sysconf_query(8, 8, 10) > 0);
}
