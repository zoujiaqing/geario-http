/* Starts an HTTP/1.1 server from C and answers every request. */
#include <stdio.h>
#include <string.h>
#include <signal.h>
#include <unistd.h>
#include "geario_http.h"

static volatile sig_atomic_t running = 1;
static void on_sigint(int sig) { (void)sig; running = 0; }

static void on_request(void *user_data, const GearioHttpRequest *req) {
    long *count = (long *)user_data;
    (*count)++;

    char body[512];
    int n = snprintf(body, sizeof body,
                     "method=%.*s path=%.*s query=%.*s n=%ld\n",
                     (int)req->method.len, (const char *)req->method.ptr,
                     (int)req->path.len,   (const char *)req->path.ptr,
                     (int)req->query.len,  (const char *)req->query.ptr,
                     *count);

    static const char headers[] = "content-type: text/plain\nx-from: c";

    GearioHttpStatus st = geario_http_respond(
        req->responder, 200,
        (const unsigned char *)headers, sizeof(headers) - 1,
        (const unsigned char *)body, (size_t)n);

    if (st != GEARIO_HTTP_STATUS_OK) {
        fprintf(stderr, "respond failed: %d\n", st);
    }
}

int main(int argc, char **argv) {
    unsigned short port = (argc > 1) ? (unsigned short)atoi(argv[1]) : 8090;
    long count = 0;

    signal(SIGINT, on_sigint);

    GearioHttpServer *srv =
        geario_http_server_start("127.0.0.1", port, on_request, &count);
    if (!srv) {
        fprintf(stderr, "server failed to start\n");
        return 1;
    }

    printf("listening on 127.0.0.1:%u\n", port);
    fflush(stdout);

    while (running) { sleep(1); }

    geario_http_server_stop(srv);
    printf("served %ld requests\n", count);
    return 0;
}
