#ifndef BLUEFIN_H
#define BLUEFIN_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Blocking: call on a background thread. Input must point to readable NUL-terminated
 * UTF-8, at most 65536 bytes excluding NUL. Never pass dangling pointers.
 * Returned UTF-8 JSON belongs to caller; release exactly once with bluefin_free.
 * NULL input returns an error envelope. NULL free is safe.
 * playback returns local url, proxyId, startOffsetTicks and original session fields.
 * Pass url directly to AVPlayer; no custom HTTP headers. On every exit/failure call
 * {"operation":"stopPlaybackProxy","proxyId":"..."} on a background thread.
 * Premium musicItems returns Items/TotalRecordCount; playlist removals take entryIds.
 * Offline operations require cacheRoot owned by this app plus server/userId. Only
 * completed downloads return localUrl. Discard offline URLs and stop players on
 * account change/logout; call downloadCancel off the main thread. Tokens are not
 * persisted. Cache roots must have no symlink path components.
 * Original report ticks = player ticks + startOffsetTicks. See Rust README. */
char *bluefin_call(const char *json);
/* Input is a readable PNG/JPEG buffer, 1..8 MiB. Output owns invitation secrets.
 * Call off the main thread and release the result with bluefin_free. */
char *bluefin_decode_qr_image(const uint8_t *input, size_t len);
void bluefin_free(char *json);
#ifdef __cplusplus
}
#endif
#endif
