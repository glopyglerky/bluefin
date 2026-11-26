import {MediaSource, SourceBufferImpl, VideoPlayer} from '@amazon-devices/react-native-w3cmedia';
import type HlsType from 'hls.js';
import {grantResource} from './media-policy.mjs';

// Public MSE porting boundary: Vega has no DOM, workers or streaming Fetch body.
// Upstream hls.js remains an Apache-2.0 dependency, never an Amazon wrapper.
export function createAdaptive(video: VideoPlayer, onError: () => void) {
  const host = globalThis as any;
  host.self = host.window = host;
  host.MediaSource = MediaSource;
  host.SourceBuffer = SourceBufferImpl;
  host.HTMLVideoElement = VideoPlayer;
  host.location ??= {href: 'https://invalid.local/'};
  host.document ??= {createElement: () => ({canPlayType: () => ''})};
  host.performance ??= {now: () => Date.now()};
  // hls.js passes BufferSource; Vega's documented port requires Uint8Array.
  class VegaSource extends MediaSource {
    addSourceBuffer(mime: string) {
      const buffer = super.addSourceBuffer(mime);
      const append = buffer.appendBuffer.bind(buffer);
      buffer.appendBuffer = data => append(data instanceof Uint8Array ? data : new Uint8Array(data as ArrayBuffer));
      return buffer;
    }
  }
  host.MediaSource = VegaSource;
  // Require after public API globals exist: upstream inspects MSE at module load.
  const Hls: typeof HlsType = require('hls.js/dist/hls.light');
  let root = '', disposed = false;
  const controllers = new Set<AbortController>();
  class GrantLoader {
    context: any; callbacks: any;
    stats = {aborted:false,loaded:0,retry:0,total:0,chunkCount:0,bwEstimate:0,
      loading:{start:0,first:0,end:0},parsing:{start:0,end:0},buffering:{start:0,first:0,end:0}};
    abort() { this.stats.aborted = true; this.controller?.abort(); }
    destroy() { this.abort(); }
    controller?: AbortController;
    getCacheAge() { return null; }
    getResponseHeader() { return null; }
    load(context: any, _config: any, callbacks: any) {
      this.context = context; this.callbacks = callbacks;
      const controller = this.controller = new AbortController(); controllers.add(controller);
      const timeout = setTimeout(() => controller.abort(),15000);
      this.stats.loading.start = host.performance.now();
      Promise.resolve().then(async () => {
        if (disposed) throw Error('Stopped');
        const url = grantResource(root,context.url);
        const options = {signal:controller.signal,credentials:'omit' as const,redirect:'error',headers:{}};
        const response = await fetch(url,options);
        if (!response.ok || response.redirected) throw Error('Media denied');
        if (response.url && response.url !== url) throw Error('Redirect denied');
        this.stats.loading.first = host.performance.now();
        const data = context.responseType === 'arraybuffer' ? await response.arrayBuffer() : await response.text();
        if (disposed || controller.signal.aborted) return;
        const size = typeof data === 'string' ? data.length : data.byteLength;
        if (size > 32*1024*1024) throw Error('Media limit');
        // Only clear media; encrypted/key-bearing playlists require separate admission.
        if (typeof data === 'string' && /#EXT-X-(SESSION-)?KEY:.*METHOD=(?!NONE)/.test(data)) throw Error('Encrypted media unavailable');
        this.stats.loaded = this.stats.total = size; this.stats.loading.end = host.performance.now();
        callbacks.onSuccess({url,data,code:response.status},this.stats,context,response);
      }).catch(() => { if (!disposed && !this.stats.aborted) callbacks.onError({code:0,text:'Media request failed'},context,null); })
        .finally(() => {clearTimeout(timeout);controllers.delete(controller);});
    }
  }
  const hls = new Hls({enableWorker:false,progressive:false,lowLatencyMode:false,
    loader:GrantLoader as any,backBufferLength:30,maxBufferLength:30});
  const media = video as any;
  media.getAttribute = (name: string) => name === 'src' ? video.src : null;
  media.removeAttribute = (name: string) => { if (name === 'src') video.src = ''; };
  media.load = () => {};
  let rejectLoad: ((error: Error) => void) | undefined;
  return {
    load(url: string): Promise<void> {
      root = grantResource(url,url);
      return new Promise((resolve,reject) => {
        rejectLoad = reject;
        hls.on(Hls.Events.ERROR,(_event,data) => { if(data.fatal) { reject(Error('HLS failed')); onError(); } });
        hls.on(Hls.Events.MANIFEST_PARSED,() => {rejectLoad = undefined;resolve();});
        hls.attachMedia(media); hls.loadSource(root);
      });
    },
    async destroy() {
      disposed = true; rejectLoad?.(Error('Stopped')); rejectLoad = undefined;
      for (const controller of controllers) controller.abort();
      controllers.clear(); hls.destroy();
    },
  };
}
