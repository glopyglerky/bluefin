#ifndef BLUEFIN_H
#define BLUEFIN_H
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
 * Original report ticks = player ticks + startOffsetTicks. See Rust README. */
char *bluefin_call(const char *json);
void bluefin_free(char *json);
#ifdef __cplusplus
}
#endif
#endif
