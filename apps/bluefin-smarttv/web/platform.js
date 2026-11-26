/* Browser/webOS public HTML media adapter. Samsung replaces this file. */
(function () {
  'use strict';
  var video, ended = function () {}, error = function () {};
  window.BluefinPlatform = {
    name: window.PalmSystem ? 'LG webOS' : 'Browser qualification',
    init: function (onEnded, onError) { video = document.getElementById('video'); ended = onEnded; error = onError; video.addEventListener('ended', ended); video.addEventListener('error', function () { error(new Error('The platform player rejected this stream. Try another negotiated format.')); }); },
    canPlay: function (kind) { return !!video.canPlayType(kind === 'audio' ? 'audio/mpeg' : 'application/vnd.apple.mpegurl'); },
    play: function (url) { video.src = url; return Promise.resolve(video.play()); },
    pause: function (paused) { if (paused === false || paused === undefined && video.paused) return Promise.resolve(video.play()); video.pause(); return Promise.resolve(); },
    seek: function (seconds) { if (isFinite(video.duration)) video.currentTime = Math.max(0, Math.min(seconds, video.duration)); },
    position: function () { return {position: video.currentTime || 0, duration: isFinite(video.duration) ? video.duration : 0, paused:video.paused}; },
    stop: function () { video.pause(); video.removeAttribute('src'); video.load(); },
    tracks: function () {
      var result = [], i;
      if (video.audioTracks) for (i = 0; i < video.audioTracks.length; i++) result.push({type:'AUDIO', index:i, label:video.audioTracks[i].label || video.audioTracks[i].language || 'Audio '+(i+1)});
      if (video.textTracks) for (i = 0; i < video.textTracks.length; i++) result.push({type:'TEXT', index:i, label:video.textTracks[i].label || video.textTracks[i].language || 'Subtitles '+(i+1)});
      return result;
    },
    selectTrack: function (track) { var i, tracks = track.type === 'AUDIO' ? video.audioTracks : video.textTracks; if (!tracks) return; for (i=0;i<tracks.length;i++) { if (track.type === 'AUDIO') tracks[i].enabled = i === track.index; else tracks[i].mode = i === track.index ? 'showing' : 'disabled'; } },
    handoff: function (url) {
      // Only call after Rust has admitted an exact public watch URL and a user
      // activates the provider button. Never pass server/account credentials.
      if (window.PalmServiceBridge) return new Promise(function (resolve, reject) {
        var bridge = new PalmServiceBridge(); window.BluefinPlatform.pendingBridge = bridge;
        bridge.onservicecallback = function (raw) { window.BluefinPlatform.pendingBridge = null; var result=JSON.parse(raw); if (result.returnValue) resolve(); else reject(new Error('Provider application could not be opened.')); };
        bridge.call('luna://com.webos.applicationManager/launch', JSON.stringify({id:'com.webos.app.browser',params:{target:url}}));
      });
      return Promise.reject(new Error('Provider handoff needs the installed webOS application.'));
    },
    exit: function () { if (window.PalmSystem) window.close(); }
  };
}());
