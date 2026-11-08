#ifndef BLUEFIN_RUST_BRIDGING_HEADER_H
#define BLUEFIN_RUST_BRIDGING_HEADER_H
char *bluefin_call(const char *request_json);
void bluefin_free(char *response_json);
#endif
