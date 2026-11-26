#ifndef BLUEFIN_RUST_BRIDGING_HEADER_H
#define BLUEFIN_RUST_BRIDGING_HEADER_H
#include <stddef.h>
#include <stdint.h>
char *bluefin_decode_qr_image(const uint8_t *image, size_t length);
char *bluefin_call(const char *request_json);
void bluefin_free(char *response_json);
#endif
