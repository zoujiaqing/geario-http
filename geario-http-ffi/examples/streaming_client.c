/* Server streams chunks; client must see them arrive one at a time. */
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include "geario_http.h"

#define PORT 8095
#define CHUNKS 5

static void on_request(void *ud, const GearioHttpRequest *req) {
    (void)ud;
    if (geario_http_response_begin(req->responder, 200, NULL, 0) != GEARIO_HTTP_STATUS_OK) {
        return;
    }
    for (int i = 1; i <= CHUNKS; i++) {
        char c[32];
        int n = snprintf(c, sizeof c, "part-%d;", i);
        geario_http_response_write(req->responder, (const unsigned char *)c, (size_t)n);
    }
    geario_http_response_finish(req->responder);
}

struct call {
    int chunks;
    char body[512];
    size_t len;
    int done;
};

static GearioHttpChunkAction on_chunk(void *ud, uint64_t id,
                                      const unsigned char *chunk, size_t len) {
    (void)id;
    struct call *c = ud;
    c->chunks++;
    size_t room = sizeof(c->body) - c->len - 1;
    size_t take = len < room ? len : room;
    memcpy(c->body + c->len, chunk, take);
    c->len += take;
    c->body[c->len] = '\0';
    return GEARIO_HTTP_CHUNK_CONTINUE;
}

static void on_done(void *ud, uint64_t id, const GearioHttpError *err) {
    (void)id;
    struct call *c = ud;
    if (err) {
        fprintf(stderr, "failed: kind=%d %.*s\n", err->kind,
                (int)err->message.len, (const char *)err->message.ptr);
    }
    c->done = 1;
}

int main(void) {
    GearioHttpServer *srv = geario_http_server_start("127.0.0.1", PORT, on_request, NULL);
    if (!srv) { fprintf(stderr, "server failed\n"); return 1; }

    GearioHttpClientOptions opts;
    geario_http_client_options_init(&opts, sizeof opts);
    GearioHttpClient *cli = NULL;
    if (geario_http_client_new(&opts, &cli) != GEARIO_HTTP_STATUS_OK) {
        fprintf(stderr, "client failed\n"); return 1;
    }

    char url[64];
    snprintf(url, sizeof url, "http://127.0.0.1:%d/", PORT);

    GearioHttpClientRequest req;
    geario_http_client_request_init(&req, sizeof req);
    req.url.ptr = (const unsigned char *)url;
    req.url.len = strlen(url);

    struct call c = {0};
    uint64_t id = 0;
    if (geario_http_client_send(cli, &req, NULL, on_chunk, on_done, &c, &id)
        != GEARIO_HTTP_STATUS_OK) {
        fprintf(stderr, "send failed\n"); return 1;
    }
    for (int spin = 0; spin < 500 && !c.done; spin++) { usleep(10000); }

    geario_http_client_free(cli);
    geario_http_server_stop(srv);

    printf("chunks=%d body=%s\n", c.chunks, c.body);

    if (!c.done)  { fprintf(stderr, "timed out\n"); return 1; }
    if (c.chunks < 2) {
        fprintf(stderr, "body arrived in %d call(s); it was buffered, not streamed\n",
                c.chunks);
        return 1;
    }
    return 0;
}
