/* Benchmark server over the geario C ABI.
 *
 * Answers every request with a fixed-size body so the load client can check
 * the response rather than only counting it. Body size from BENCH_BODY_SIZE,
 * address from BENCH_ADDR, to match the other bench servers.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <signal.h>
#include <unistd.h>
#include "geario_http.h"

static volatile sig_atomic_t running = 1;
static void on_sigint(int sig) { (void)sig; running = 0; }

static unsigned char *g_body;
static size_t g_body_len;

static void on_request(void *user_data, const GearioHttpRequest *req) {
    (void)user_data;
    static const char headers[] = "content-type: application/json";
    geario_http_respond(req->responder, 200,
                        (const unsigned char *)headers, sizeof(headers) - 1,
                        g_body, g_body_len);
}

int main(void) {
    const char *addr = getenv("BENCH_ADDR");
    const char *host = "127.0.0.1";
    unsigned short port = 18099;
    char hostbuf[64];
    if (addr) {
        const char *colon = strrchr(addr, ':');
        if (colon) {
            size_t n = (size_t)(colon - addr);
            if (n >= sizeof hostbuf) n = sizeof hostbuf - 1;
            memcpy(hostbuf, addr, n);
            hostbuf[n] = 0;
            host = hostbuf;
            port = (unsigned short)atoi(colon + 1);
        }
    }

    const char *bs = getenv("BENCH_BODY_SIZE");
    g_body_len = bs ? (size_t)strtoul(bs, NULL, 10) : 24;
    g_body = malloc(g_body_len ? g_body_len : 1);
    const char seed[] = "hello from the benchmark ";
    for (size_t i = 0; i < g_body_len; i++) g_body[i] = seed[i % (sizeof seed - 1)];

    signal(SIGINT, on_sigint);
    GearioHttpServer *srv = geario_http_server_start(host, port, on_request, NULL);
    if (!srv) { fprintf(stderr, "server failed to start\n"); return 1; }
    printf("ffi bench server on %s:%u body=%zu\n", host, port, g_body_len);
    fflush(stdout);
    while (running) sleep(1);
    geario_http_server_stop(srv);
    return 0;
}
