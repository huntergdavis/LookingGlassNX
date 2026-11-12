#ifndef LOOKINGGLASSNX_SYSCONF_COMPAT_H
#define LOOKINGGLASSNX_SYSCONF_COMPAT_H

#include <cerrno>

inline long lgnx_sysconf_query(int name, int page_size_name, int processors_online_name) {
    if (name == page_size_name) return 4096;
    if (name == processors_online_name) return 1;
    errno = EINVAL;
    return -1;
}

#endif
