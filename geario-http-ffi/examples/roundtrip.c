/* Starts a server and drives it with the client, all from C. */
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include "geario_http.h"

#define PORT 8094

/* ---- server side ---- */
static void on_request(void *user_data, const GearioHttpRequest *req) {
    (void)user_data;
    char body[256];
    int n = snprintf(body, sizeof body, "served %.*s",
                     (int)req->path.len, (const char *)req->path.ptr);
    geario_http_respond(req->responder, 200, NULL, 0,
                        (const unsigned char *)body, (size_t)n);
}

/* ---- client side ---- */
struct call {
    int status;
    char body[512];
    size_t body_len;
    int done;
    int header_count;
};

static GearioHttpHeadersAction on_headers(void *ud, uint64_t id, uint16_t status,
                                          uint8_t version,
                                          const GearioHttpHeader *hdrs, size_t n) {
    (void)id; (void)version; (void)hdrs;
    struct call *c = ud;
    c->status = status;
    c->header_count = (int)n;
    return GEARIO_HTTP_HEADERS_CONTINUE;
}

static GearioHttpChunkAction on_chunk(void *ud, uint64_t id,
                                      const unsigned char *chunk, size_t len) {
    (void)id;
    struct call *c = ud;
    size_t room = sizeof(c->body) - c->body_len - 1;
    size_t take = len < room ? len : room;
    memcpy(c->body + c->body_len, chunk, take);
    c->body_len += take;
    c->body[c->body_len] = '\0';
    return GEARIO_HTTP_CHUNK_CONTINUE;
}

static void on_done(void *ud, uint64_t id, const GearioHttpError *err) {
    (void)id;
    struct call *c = ud;
    if (err) {
        fprintf(stderr, "request failed: kind=%d %.*s\n", err->kind,
                (int)err->message.len, (const char *)err->message.ptr);
    }
    c->done = 1;
}

int main(void) {
    GearioHttpServer *srv =
        geario_http_server_start("127.0.0.1", PORT, on_request, NULL);
    if (!srv) { fprintf(stderr, "server failed\n"); return 1; }

    GearioHttpClientOptions opts;
    GearioHttpStatus st = geario_http_client_options_init(&opts, sizeof opts);
    if (st != GEARIO_HTTP_STATUS_OK) { fprintf(stderr, "options: %d\n", st); return 1; }

    GearioHttpClient *cli = NULL;
    st = geario_http_client_new(&opts, &cli);
    if (st != GEARIO_HTTP_STATUS_OK) { fprintf(stderr, "client_new: %d\n", st); return 1; }

    const char *paths[] = {"/alpha", "/beta", "/gamma"};
    int failures = 0;

    for (int i = 0; i < 3; i++) {
        char url[128];
        snprintf(url, sizeof url, "http://127.0.0.1:%d%s", PORT, paths[i]);

        GearioHttpClientRequest req;
        geario_http_client_request_init(&req, sizeof req);
        req.method.ptr = (const unsigned char *)"GET";
        req.method.len = 3;
        req.url.ptr = (const unsigned char *)url;
        req.url.len = strlen(url);

        struct call c = {0};
        uint64_t id = 0;
        st = geario_http_client_send(cli, &req, on_headers, on_chunk, on_done, &c, &id);
        if (st != GEARIO_HTTP_STATUS_OK) {
            fprintf(stderr, "send: %d\n", st); failures++; continue;
        }

        for (int spin = 0; spin < 500 && !c.done; spin++) { usleep(10000); }

        if (!c.done)            { fprintf(stderr, "%s timed out\n", paths[i]); failures++; }
        else if (c.status != 200) { fprintf(stderr, "%s status %d\n", paths[i], c.status); failures++; }
        else printf("id=%llu %s -> %d (%d headers) %s\n",
                    (unsigned long long)id, paths[i], c.status, c.header_count, c.body);
    }

    printf("inflight=%u paused=%u\n",
           geario_http_client_inflight_count(cli),
           geario_http_client_paused_stream_count(cli));

    geario_http_client_free(cli);
    geario_http_server_stop(srv);
    return failures ? 1 : 0;
}
