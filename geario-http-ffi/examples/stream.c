/* Streams a chunked response from C. */
#include <stdio.h>
#include <string.h>
#include <signal.h>
#include <unistd.h>
#include "geario_http.h"

static volatile sig_atomic_t running = 1;
static void on_sigint(int sig) { (void)sig; running = 0; }

static void on_request(void *user_data, const GearioHttpRequest *req) {
    (void)user_data;

    static const char headers[] = "content-type: text/plain";
    GearioHttpStatus st = geario_http_response_begin(
        req->responder, 200,
        (const unsigned char *)headers, sizeof(headers) - 1);
    if (st != GEARIO_HTTP_STATUS_OK) {
        fprintf(stderr, "begin failed: %d\n", st);
        return;
    }

    /* Answering a streaming responder one-shot must be refused. */
    GearioHttpStatus bad = geario_http_respond(
        req->responder, 500, NULL, 0, (const unsigned char *)"x", 1);
    if (bad != GEARIO_HTTP_STATUS_WRONG_STATE) {
        fprintf(stderr, "expected WRONG_STATE, got %d\n", bad);
    }

    for (int i = 1; i <= 5; i++) {
        char chunk[64];
        int n = snprintf(chunk, sizeof chunk, "chunk %d\n", i);
        st = geario_http_response_write(req->responder,
                                        (const unsigned char *)chunk, (size_t)n);
        if (st != GEARIO_HTTP_STATUS_OK) {
            fprintf(stderr, "write %d failed: %d\n", i, st);
            return;
        }
    }

    st = geario_http_response_finish(req->responder);
    if (st != GEARIO_HTTP_STATUS_OK) {
        fprintf(stderr, "finish failed: %d\n", st);
    }
}

int main(int argc, char **argv) {
    unsigned short port = (argc > 1) ? (unsigned short)atoi(argv[1]) : 8093;
    signal(SIGINT, on_sigint);

    GearioHttpServer *srv =
        geario_http_server_start("127.0.0.1", port, on_request, NULL);
    if (!srv) { fprintf(stderr, "server failed to start\n"); return 1; }

    printf("streaming on 127.0.0.1:%u\n", port);
    fflush(stdout);
    while (running) { sleep(1); }

    geario_http_server_stop(srv);
    return 0;
}
