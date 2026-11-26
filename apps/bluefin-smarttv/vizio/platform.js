/* Public HTMLMediaElement adapter. No undocumented VIZIO bridge calls. */
(function () {
  'use strict';
  var video, onError, generation = 0;
  function fault() { onError(new Error('The TV player could not play this negotiated stream.')); }
  window.BluefinPlatform = {
    name: 'Vizio hosted candidate',
    init: function (ended, error) {
      video = document.getElementById('video'); onError = error;
      video.addEventListener('ended', ended);
      video.addEventListener('error', fault);
      // The common app owns grant release on pagehide/visibilitychange.
      window.addEventListener('pagehide', function () { window.BluefinPlatform.stop(); });
    },
    canPlay: function (kind) { return !!video.canPlayType(kind === 'audio' ? 'audio/mpeg' : 'application/vnd.apple.mpegurl'); },
    play: function (url) {
      if (document.hidden) return Promise.reject(new Error('Return to Bluefin and select Play.'));
      // Defense in depth: native media receives an opaque capability only.
      var parsed = new URL(url);
      if (parsed.protocol !== 'https:' || parsed.username || parsed.password || parsed.search || parsed.hash || !/\/Bluefin\/TV\/v1\/media\/[a-f0-9]{64}\/\d+$/.test(parsed.pathname)) return Promise.reject(new Error('Invalid player grant.'));
      var own = ++generation; video.src = url;
      return Promise.resolve(video.play()).then(function () { if (own !== generation || document.hidden) throw new Error('Playback was cancelled.'); });
    },
    pause: function (paused) { if (paused === false || paused === undefined && video.paused) return Promise.resolve(video.play()); video.pause(); return Promise.resolve(); },
    seek: function (seconds) { if (Number.isFinite(seconds) && Number.isFinite(video.duration)) video.currentTime = Math.max(0, Math.min(seconds, video.duration)); },
    position: function () { return {position: video.currentTime || 0, duration: Number.isFinite(video.duration) ? video.duration : 0, paused: video.paused}; },
    stop: function () { generation++; if (video) { video.pause(); video.removeAttribute('src'); video.load(); } },
    tracks: function () {
      var result = [];
      ['audioTracks', 'textTracks'].forEach(function (key) { var list = video[key]; if (list) for (var i = 0; i < list.length; i++) result.push({type: key === 'audioTracks' ? 'AUDIO' : 'TEXT', index: i, label: list[i].label || list[i].language || String(i + 1)}); });
      return result;
    },
    selectTrack: function (track) { var list = track.type === 'AUDIO' ? video.audioTracks : video.textTracks; if (!list || !Number.isInteger(track.index) || track.index < 0 || track.index >= list.length) return; for (var i = 0; i < list.length; i++) { if (track.type === 'AUDIO') list[i].enabled = i === track.index; else list[i].mode = i === track.index ? 'showing' : 'disabled'; } },
    handoff: function () { return Promise.reject(new Error('Provider launch requires the approved Vizio partner interface.')); },
    exit: function () { document.getElementById('status').textContent = 'Use the TV Home control to leave Bluefin.'; }
  };
}());
