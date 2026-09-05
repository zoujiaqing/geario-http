/* Hand-written for now; run ./gen_header.sh once cbindgen is installed. */
#ifndef GEARIO_HTTP_H
#define GEARIO_HTTP_H

#include <stdint.h>

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

#ifdef __cplusplus
}
#endif

#endif /* GEARIO_HTTP_H */
