#include <stdio.h>
#include "geario_http.h"

/* Prints what this build can do. Server and client have separate bit sets. */
int main(void) {
    printf("geario-http %s, abi %u.%u\n", geario_http_version(),
           geario_http_abi_version() >> 16, geario_http_abi_version() & 0xffff);

    uint64_t s = geario_http_server_capabilities();
    printf("server:");
    if (s & GEARIO_HTTP_SERVER_CAP_HTTP1)     printf(" http1");
    if (s & GEARIO_HTTP_SERVER_CAP_H2C)       printf(" h2c");
    if (s & GEARIO_HTTP_SERVER_CAP_STREAMING) printf(" streaming");
    printf("\n");

    uint64_t c = geario_http_client_capabilities();
    printf("client:");
    if (c & GEARIO_HTTP_CLIENT_CAP_HTTP1)     printf(" http1");
    if (c & GEARIO_HTTP_CLIENT_CAP_HTTP2)     printf(" http2");
    if (c & GEARIO_HTTP_CLIENT_CAP_TLS)       printf(" tls");
    if (c & GEARIO_HTTP_CLIENT_CAP_CUSTOM_CA) printf(" custom-ca");
    if (c & GEARIO_HTTP_CLIENT_CAP_CANCEL)    printf(" cancel");
    if (c & GEARIO_HTTP_CLIENT_CAP_STREAMING) printf(" streaming");
    if (c & GEARIO_HTTP_CLIENT_CAP_PROXY)     printf(" proxy");
    printf("\n");
    return 0;
}
