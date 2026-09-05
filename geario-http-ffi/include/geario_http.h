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

#ifdef __cplusplus
}
#endif

#endif /* GEARIO_HTTP_H */
