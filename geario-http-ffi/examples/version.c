/* Links libgeario_http_ffi and asks it what it is. */
#include <stdio.h>
#include "geario_http.h"

#define CAP_HTTP1     GEARIO_HTTP_CAP_HTTP1
#define CAP_HTTP2     GEARIO_HTTP_CAP_HTTP2
#define CAP_TLS       GEARIO_HTTP_CAP_TLS
#define CAP_STREAMING GEARIO_HTTP_CAP_STREAMING

static void print_caps(const char *what, uint64_t caps) {
    printf("%-8s caps=0x%llx", what, (unsigned long long)caps);
    if (caps == 0) { printf(" (not built in)"); }
    if (caps & CAP_HTTP1)     printf(" http1");
    if (caps & CAP_HTTP2)     printf(" http2");
    if (caps & CAP_TLS)       printf(" tls");
    if (caps & CAP_STREAMING) printf(" streaming");
    printf("\n");
}

int main(void) {
    printf("abi      %u\n", geario_http_abi_version());
    printf("version  %s\n", geario_http_version());
    print_caps("server", geario_http_server_capabilities());
    print_caps("client", geario_http_client_capabilities());
    return 0;
}
