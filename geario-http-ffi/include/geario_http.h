/* Hand-written for now; run ./gen_header.sh once cbindgen is installed. */
#ifndef GEARIO_HTTP_H
#define GEARIO_HTTP_H

#include <stdint.h>
#include <stdlib.h>

#ifdef __cplusplus
extern "C" {
#endif

/** Synchronous status code. Fixed width: a C enum's width is
 *  implementation-defined, so freezing the values would freeze nothing. */
typedef int32_t GearioHttpStatus;

#define GEARIO_HTTP_STATUS_OK            0
#define GEARIO_HTTP_STATUS_ABI_MISMATCH  (-1)
#define GEARIO_HTTP_STATUS_STRUCT_SIZE   (-2)
#define GEARIO_HTTP_STATUS_UNKNOWN_FLAGS (-3)
#define GEARIO_HTTP_STATUS_INVALID_ARG   (-4)
#define GEARIO_HTTP_STATUS_UNSUPPORTED   (-5)
#define GEARIO_HTTP_STATUS_WRONG_THREAD  (-6)
#define GEARIO_HTTP_STATUS_CLOSED        (-7)
#define GEARIO_HTTP_STATUS_OOM           (-8)
#define GEARIO_HTTP_STATUS_WRONG_STATE   (-9)
#define GEARIO_HTTP_STATUS_THROTTLED     (-10)

/** Capability bits. Derived from cargo features, so a bit cannot claim
 *  something this build does not contain. */
#define GEARIO_HTTP_CAP_HTTP1     (UINT64_C(1) << 0)
#define GEARIO_HTTP_CAP_HTTP2     (UINT64_C(1) << 1)
#define GEARIO_HTTP_CAP_TLS       (UINT64_C(1) << 2)
#define GEARIO_HTTP_CAP_STREAMING (UINT64_C(1) << 3)

/** ABI revision of this build. */
uint32_t geario_http_abi_version(void);

/** NUL-terminated crate version. Static storage; do not free. */
const char *geario_http_version(void);

/** What the server side of this build can do. Zero means the server was not
 *  compiled in, which is the only way to tell that apart from a call that
 *  merely failed. */
uint64_t geario_http_server_capabilities(void);

/** What the client side of this build can do. */
uint64_t geario_http_client_capabilities(void);

/* ------------------------------------------------------------------ */
/* Server                                                             */
/* ------------------------------------------------------------------ */

/** A borrowed view of bytes geario owns. Valid only for the duration of the
 *  callback it arrives in; copy anything you need to keep. */
typedef struct {
    const unsigned char *ptr;
    size_t len;
} GearioHttpSlice;

/** A request handed to the host. `responder` outlives the callback. */
typedef struct {
    GearioHttpSlice method;
    GearioHttpSlice path;
    GearioHttpSlice query;
    /** Headers as `name: value` lines separated by '\n'. */
    GearioHttpSlice headers;
    GearioHttpSlice body;
    uint64_t responder;
} GearioHttpRequest;

/** Called once per request, on the worker owning the connection. Different
 *  connections land on different workers, so this must be safe to call
 *  concurrently. */
typedef void (*GearioHttpRequestCallback)(void *user_data,
                                          const GearioHttpRequest *req);

/** Opaque server handle. */
typedef struct GearioHttpServer GearioHttpServer;

/** Start an HTTP/1.1 server. Returns NULL on a bad address or if the worker
 *  thread cannot start. `user_data` must outlive the server. */
GearioHttpServer *geario_http_server_start(const char *host,
                                           uint16_t port,
                                           GearioHttpRequestCallback on_request,
                                           void *user_data);

/** Answer a request.
 *
 *  Must be called on the worker that delivered the responder. Calling it from
 *  another thread returns GEARIO_HTTP_STATUS_WRONG_THREAD: geario is
 *  thread-per-core and its handles are not shared between workers.
 *
 *  `headers` is `name: value` lines separated by '\n', or NULL. */
GearioHttpStatus geario_http_respond(uint64_t responder,
                                     uint16_t status,
                                     const unsigned char *headers,
                                     size_t headers_len,
                                     const unsigned char *body,
                                     size_t body_len);

/** Send status and headers now and stream the body afterwards.
 *
 *  Follow with geario_http_response_write per chunk and
 *  geario_http_response_finish at the end. Calling geario_http_respond on a
 *  responder that is already streaming returns GEARIO_HTTP_STATUS_WRONG_STATE,
 *  and so does the reverse. */
GearioHttpStatus geario_http_response_begin(uint64_t responder,
                                            uint16_t status,
                                            const unsigned char *headers,
                                            size_t headers_len);

/** Append one chunk. The bytes are copied before this returns. */
GearioHttpStatus geario_http_response_write(uint64_t responder,
                                            const unsigned char *chunk,
                                            size_t chunk_len);

/** Close a streaming body. */
GearioHttpStatus geario_http_response_finish(uint64_t responder);

/** Stop a server and free its handle. NULL is a no-op. */
void geario_http_server_stop(GearioHttpServer *server);

/* ------------------------------------------------------------------ */
/* Client                                                             */
/* ------------------------------------------------------------------ */

/** What a callback wants to happen next. Headers and chunks use separate
 *  types: "pause before the next chunk" has no meaning at the headers stage,
 *  and one shared enum would leave that combination undefined. */
typedef int32_t GearioHttpHeadersAction;
#define GEARIO_HTTP_HEADERS_CONTINUE 0
#define GEARIO_HTTP_HEADERS_CANCEL   2

typedef int32_t GearioHttpChunkAction;
#define GEARIO_HTTP_CHUNK_CONTINUE 0
#define GEARIO_HTTP_CHUNK_PAUSE    1
#define GEARIO_HTTP_CHUNK_CANCEL   2

typedef int32_t GearioHttpErrorKind;
#define GEARIO_HTTP_ERR_NONE        0
#define GEARIO_HTTP_ERR_CONNECT     1
#define GEARIO_HTTP_ERR_TIMEOUT     2
#define GEARIO_HTTP_ERR_PROTOCOL    3
#define GEARIO_HTTP_ERR_IO          4
#define GEARIO_HTTP_ERR_CANCELLED   5
#define GEARIO_HTTP_ERR_INVALID_URL 6
#define GEARIO_HTTP_ERR_UNSUPPORTED 7

typedef struct {
    GearioHttpSlice name;
    GearioHttpSlice value;
} GearioHttpHeader;

typedef struct {
    GearioHttpErrorKind kind;
    uint32_t protocol_code;
    /** Borrowed diagnostic text. For logs only, never branch on it. */
    GearioHttpSlice message;
} GearioHttpError;

typedef struct {
    uint32_t abi_version;
    uint32_t struct_size;
    uint64_t flags;
    /** 0 disables the connect timeout. Not "use the default", not "expire
     *  immediately" - both readings exist in the wild, so this one is pinned. */
    uint64_t connect_timeout_ms;
    /** 0 disables the overall timeout, which streaming responses need. */
    uint64_t request_timeout_ms;
    /** Ceiling on requests in flight. 0 uses the built-in default. Exceeding
     *  it returns GEARIO_HTTP_STATUS_THROTTLED rather than queueing. */
    uint32_t max_inflight_requests;
    /** *Additional* attempts: 0 means try once, 2 means at most three tries.
     *
     *  Only idempotent methods are retried, and only when the failure happened
     *  before a response started. Retrying a POST that may already have been
     *  applied is a correctness bug, not a resilience feature. */
    uint32_t max_retries;
} GearioHttpClientOptions;

typedef struct {
    uint32_t abi_version;
    uint32_t struct_size;
    GearioHttpSlice method;
    GearioHttpSlice url;
    const GearioHttpHeader *headers;
    size_t header_count;
    const unsigned char *body;
    size_t body_len;
} GearioHttpClientRequest;

typedef struct GearioHttpClient GearioHttpClient;

typedef GearioHttpHeadersAction (*GearioHttpOnHeaders)(
    void *user_data, uint64_t request_id, uint16_t status,
    uint8_t http_version, const GearioHttpHeader *headers, size_t header_count);

typedef GearioHttpChunkAction (*GearioHttpOnChunk)(
    void *user_data, uint64_t request_id,
    const unsigned char *chunk, size_t chunk_len);

typedef void (*GearioHttpOnDone)(
    void *user_data, uint64_t request_id, const GearioHttpError *error);

/** Fill an options struct with defaults. Pass sizeof(your struct) so an older
 *  caller only gets the prefix it allocated. */
GearioHttpStatus geario_http_client_options_init(GearioHttpClientOptions *opts,
                                                 uint32_t struct_size);

/** Fill a request struct with defaults. */
GearioHttpStatus geario_http_client_request_init(GearioHttpClientRequest *req,
                                                 uint32_t struct_size);

GearioHttpStatus geario_http_client_new(const GearioHttpClientOptions *opts,
                                        GearioHttpClient **out_client);

/** Send a request. Returns immediately; the callbacks report progress.
 *  `user_data` must stay alive until on_done returns. */
GearioHttpStatus geario_http_client_send(GearioHttpClient *client,
                                         const GearioHttpClientRequest *request,
                                         GearioHttpOnHeaders on_headers,
                                         GearioHttpOnChunk on_chunk,
                                         GearioHttpOnDone on_done,
                                         void *user_data,
                                         uint64_t *out_request_id);

GearioHttpStatus geario_http_client_cancel(GearioHttpClient *client, uint64_t request_id);
GearioHttpStatus geario_http_client_resume(GearioHttpClient *client, uint64_t request_id);
uint32_t geario_http_client_inflight_count(GearioHttpClient *client);
uint32_t geario_http_client_paused_stream_count(GearioHttpClient *client);
void geario_http_client_close(GearioHttpClient *client);
void geario_http_client_free(GearioHttpClient *client);

#ifdef __cplusplus
}
#endif

#endif /* GEARIO_HTTP_H */
