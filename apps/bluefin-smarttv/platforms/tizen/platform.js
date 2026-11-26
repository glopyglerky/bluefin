/* Samsung TV public AVPlay and TVInputDevice APIs. No DRM calls. */
(function () {
  'use strict';
  var player, ended, failed, suspended=false;
  function ready() { if (!player) throw new Error('Samsung AVPlay is unavailable on this runtime.'); }
  window.BluefinPlatform = {
    name:'Samsung Tizen',
    init:function(onEnded,onError) {
      ended=onEnded; failed=onError;
      if (!window.webapis || !webapis.avplay) throw new Error('This package requires a Samsung TV runtime.');
      player=webapis.avplay;
      var object=document.createElement('object'); object.id='avplayer';object.type='application/avplayer';document.getElementById('playerHost').appendChild(object);document.getElementById('video').hidden=true;
      if (window.tizen && tizen.tvinputdevice) ['MediaPlayPause','MediaPlay','MediaPause','MediaStop','MediaRewind','MediaFastForward','MediaTrackNext','MediaTrackPrevious'].forEach(function(key){try{tizen.tvinputdevice.registerKey(key);}catch(e){/* Mandatory/unsupported keys still use standard navigation. */}});
    },
    canPlay:function(){return !!player;},
    play:function(url) { ready();return new Promise(function(resolve,reject){try{player.open(url);player.setDisplayRect(0,0,1920,1080);player.setDisplayMethod('PLAYER_DISPLAY_MODE_LETTER_BOX');player.setListener({onstreamcompleted:ended,onerror:function(){failed(new Error('Samsung player could not play the negotiated stream.'));},onresourceconflicted:function(){failed(new Error('Playback resources are unavailable.'));}});player.prepareAsync(function(){player.play();resolve();},function(){reject(new Error('Samsung player preparation failed.'));});}catch(e){reject(e);}});},
    pause:function(paused){ready();var state=player.getState();if(state==='PLAYING'&&paused!==false)player.pause();else if(state==='PAUSED'&&paused!==true)player.play();return Promise.resolve();},
    seek:function(seconds){ready();var state=player.getState();if(state==='PLAYING'||state==='PAUSED')player.seekTo(Math.max(0,Math.min(seconds*1000,player.getDuration())));},
    position:function(){if(!player)return {position:0,duration:0};try{return {position:player.getCurrentTime()/1000,duration:player.getDuration()/1000,paused:player.getState()==='PAUSED'};}catch(e){return {position:0,duration:0};}},
    stop:function(){if(player){try{player.stop();}catch(e){}try{player.close();}catch(e){}}},
    tracks:function(){ready();try{return player.getTotalTrackInfo().filter(function(t){return t.type==='AUDIO'||t.type==='TEXT';}).map(function(t){var info={};try{info=JSON.parse(t.extra_info);}catch(e){}return {type:t.type,index:t.index,label:info.language||info.track_lang||t.type+' '+t.index};});}catch(e){return [];}},
    selectTrack:function(track){ready();player.setSelectTrack(track.type,track.index);},
    handoff:function(url){return new Promise(function(resolve,reject){if(!window.tizen)return reject(new Error('Tizen application control is unavailable.'));var control=new tizen.ApplicationControl('http://tizen.org/appcontrol/operation/view',url);tizen.application.launchAppControl(control,null,resolve,function(){reject(new Error('No supported provider application accepted this link.'));});});},
    exit:function(){if(window.tizen)tizen.application.getCurrentApplication().exit();}
  };
}());
