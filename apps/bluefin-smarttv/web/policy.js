/* Original JS host for the bounded raw Rust/WASM ABI. No auth state in WASM. */
(function () {
  'use strict';
  if (!Promise.prototype.finally) Promise.prototype.finally = function (callback) { var P = this.constructor; return this.then(function (value) { return P.resolve(callback()).then(function () { return value; }); }, function (error) { return P.resolve(callback()).then(function () { throw error; }); }); };
  var exports;
  window.BluefinPolicy = {
    load: function () {
      if (!window.WebAssembly || !window.TextEncoder || !window.TextDecoder) return Promise.reject(new Error('This TV web engine cannot run Bluefin shared policy. A newer TV runtime is required.'));
      return fetch('bluefin_tv_playback.wasm').then(function (r) { if (!r.ok) throw new Error('Shared playback policy is missing from this package.'); return r.arrayBuffer(); })
        .then(function (bytes) { return WebAssembly.instantiate(bytes, {}); }).then(function (instance) { exports = instance.instance.exports; });
    },
    apply: function (command) {
      if (!exports) throw new Error('Shared policy is not ready.');
      var bytes = new TextEncoder().encode(JSON.stringify(command));
      if (bytes.length > 262144) throw new Error('Policy input limit.');
      var pointer = exports.tv_alloc(bytes.length);
      if (!pointer) throw new Error('Policy allocation failed.');
      try {
        new Uint8Array(exports.memory.buffer, pointer, bytes.length).set(bytes);
        var resultPointer = exports.tv_apply(pointer, bytes.length);
        var result = JSON.parse(new TextDecoder().decode(new Uint8Array(exports.memory.buffer, resultPointer, exports.tv_result_len())));
        if (result.error) throw new Error(result.error); return result.value;
      } finally { exports.tv_free(pointer, bytes.length); }
    }
  };
}());
