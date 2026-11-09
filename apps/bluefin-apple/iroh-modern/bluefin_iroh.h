#ifndef BLUEFIN_IROH_H
#define BLUEFIN_IROH_H
#include <stdint.h>
#include <stddef.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Background-thread synchronous calls. Input is UTF-8 JSON, max 16384 bytes.
   {"peer_id":"<pinned public key>","peer_address":"<numeric IP:port>",
    "authorization":"Bearer <viewer token>"}
   Returns {"ok":true,"id":<uint64>,"baseUrl":"http://127.0.0.1:port/cap"}
   or a generic error. Never log input or capability URLs. Memory-only credentials.
   Append an allowed /Videos/item/... or /Audio/item/... request path to baseUrl.
   The local player sends GET/HEAD and optional single Range, no Authorization.
   Call stop on exit/replacement/error. It cancels all task sockets and joins.
   Start can fail busy. Max two tunnels, four active local streams globally. */
char *bluefin_iroh_start(const uint8_t *input, size_t len);
void bluefin_iroh_stop(uint64_t id);
void bluefin_iroh_free(char *result);
#ifdef __cplusplus
}
#endif
#endif
