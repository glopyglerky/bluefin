(function () {
  'use strict';
  var policy=window.BluefinPolicy, platform=window.BluefinPlatform;
  var content=document.getElementById('content'), status=document.getElementById('status');
  var leaseAvailable=false;
  var server='', token='', user='', lease=null, current=null, viewName='library', page=0, busy=false, epoch=0;
  var queue={queue:[],cursor:0,positionTicks:0}, queueRevision=null, lastReport=0, requestControllers=[];
  var device;
  try { device=localStorage.getItem('bluefin.tv.device'); if(!device){var bytes=new Uint8Array(16);crypto.getRandomValues(bytes);device=Array.prototype.map.call(bytes,function(b){return ('0'+b.toString(16)).slice(-2);}).join('');localStorage.setItem('bluefin.tv.device',device);} } catch(e){device='tv-'+Date.now().toString(16);}
  function message(text){status.textContent=text;}
  function button(text,action,primary){var node=document.createElement('button');node.textContent=text;if(primary)node.className='primary';node.onclick=function(){guard(action);};return node;}
  function el(tag,text,cls){var node=document.createElement(tag);if(text!==undefined)node.textContent=text;if(cls)node.className=cls;return node;}
  function guard(action){try{Promise.resolve(action()).catch(showError);}catch(e){showError(e);}}
  function showError(e){message(e.message||'The server request failed.');}
  function header(){return 'MediaBrowser Client="Bluefin TV", Device="'+platform.name+'", DeviceId="'+device+'", Version="0.1.0"'+(token?', Token="'+token+'"':'');}
  function endpoint(path){return server+path;}
  function request(path,method,body,keepalive){
    var controller=window.AbortController?new AbortController():null, timeout=setTimeout(function(){if(controller)controller.abort();},15000);
    if(controller)requestControllers.push(controller);
    return fetch(endpoint(path),{method:method||'GET',headers:{'Authorization':header(),'Content-Type':'application/json'},body:body===undefined?undefined:JSON.stringify(body),credentials:'omit',cache:'no-store',redirect:'error',referrerPolicy:'no-referrer',signal:controller?controller.signal:undefined,keepalive:!!keepalive})
      .then(function(r){if(r.status===401){token='';throw new Error('Your session expired. Sign in again.');}if(r.status===404){var missing=new Error('This server does not support this Bluefin feature.');missing.status=404;throw missing;}if(!r.ok)throw new Error('Server refused the request ('+r.status+').');if(r.status===204)return null;var length=Number(r.headers.get('content-length'));if(length>2097152)throw new Error('Server response is too large.');return r.text().then(function(text){if(text.length>2097152)throw new Error('Server response is too large.');return text?JSON.parse(text):null;});})
      .finally(function(){clearTimeout(timeout);if(controller){var i=requestControllers.indexOf(controller);if(i>=0)requestControllers.splice(i,1);}});
  }
  function grantURL(path,release){var pattern=release?/^\/Bluefin\/TV\/v1\/leases\/[a-f0-9]{64}$/:/^\/Bluefin\/TV\/v1\/media\/[a-f0-9]{64}\/\d+$/;if(!pattern.test(path))throw new Error('Server returned an unsafe playback route.');return endpoint(path);}
  var accountMode = '';
  var challenge = null;
  var choices;
  function node(id) { return document.getElementById(id); }
  function accountOptions() {
    return request('/Bluefin/Accounts/Options').catch(function(error) {
      return request('/QuickConnect/Enabled').then(function(enabled) {
        return {quickConnect: enabled === true};
      }).catch(function() { return {}; });
    });
  }
  function selectAccountMode(mode) {
    accountMode = mode;
    node('accountChoices').hidden = mode !== '';
    node('signin').hidden = mode !== 'password' && mode !== 'signup';
    node('deviceChallenge').hidden = mode !== 'passkey' && mode !== 'quickConnect';
    node('password').value = '';
    node('password').autocomplete = mode === 'signup' ? 'new-password' : 'current-password';
    node('password').minLength = mode === 'signup' ? 15 : 0;
    node('credentialSubmit').textContent = mode === 'signup' ? 'Create account' : 'Sign in';
    if (!node('signin').hidden) node('username').focus();
  }
  function showAccountOptions(advertised) {
    choices = advertised || {};
    node('signupChoice').hidden = choices.allowSignups !== true;
    node('passkeyChoice').hidden = choices.passkeys !== true;
    node('codeChoice').hidden = choices.quickConnect !== true;
    selectAccountMode('');
    node('passwordChoice').focus();
  }
  function connect(event) {
    event.preventDefault();
    if (busy) return;
    guard(function() {
      var url = new URL(node('server').value.trim());
      var qualification = url.protocol === 'http:' && url.origin === location.origin &&
        /^(localhost|127\.0\.0\.1)$/.test(url.hostname) && platform.name === 'Browser qualification';
      if ((url.protocol !== 'https:' && !qualification) || url.username || url.password || url.search || url.hash) {
        throw new Error('Use an HTTPS server address.');
      }
      server = url.origin + url.pathname.replace(/\/$/, '');
      busy = true;
      message('Connecting…');
      return request('/System/Info/Public').then(function(info) {
        if (!info || (!info.Id && !info.ServerName)) throw new Error('The address did not return media server information.');
        node('serverName').textContent = info.ServerName || 'Sign in';
        return accountOptions();
      }).then(function(options) {
        node('connect').hidden = true;
        node('account').hidden = false;
        showAccountOptions(options);
        message('');
      }).finally(function() { busy = false; });
    });
  }
  function acceptLogin(result) {
    if (!result || typeof result.AccessToken !== 'string' || !result.User ||
        !/^[a-zA-Z0-9_-]{1,512}$/.test(result.AccessToken)) {
      throw new Error('Invalid sign-in response.');
    }
    token = result.AccessToken;
    user = result.User.Id;
    return request('/Bluefin/TV/v1').catch(function(error) {
      if (error.status === 404) return null;
      throw error;
    }).then(function(caps) {
      leaseAvailable = !!(caps && caps.version === 1 && caps.mediaLease);
      node('nav').hidden = false;
      message(leaseAvailable ? '' : 'Playback is unavailable on this server.');
      return show('library');
    });
  }
  function signIn(event) {
    event.preventDefault();
    if (busy) return;
    var username = node('username').value.trim();
    var password = node('password').value;
    node('password').value = '';
    var signup = accountMode === 'signup';
    busy = true;
    message(signup ? 'Creating account…' : 'Signing in…');
    guard(function() {
      var registration = signup
        ? request('/Bluefin/Accounts/Create', 'POST', {username: username, password: password})
            .then(function() { selectAccountMode('password'); })
        : Promise.resolve();
      return registration.then(function() {
        return request('/Users/AuthenticateByName', 'POST', {Username: username, Pw: password});
      }).then(acceptLogin).catch(function(error) {
        token = '';
        if (!signup) throw error;
        return accountOptions().then(function(options) {
          showAccountOptions(options);
          throw error;
        });
      }).finally(function() { busy = false; });
    });
  }
  function cancelChallenge() {
    var running = challenge;
    challenge = null;
    if (running && running.proof) {
      return request('/Bluefin/Accounts/Passkey/Cancel', 'POST', running.proof).catch(showError);
    }
    return Promise.resolve();
  }
  function beginChallenge(mode) {
    if (busy || challenge) return;
    selectAccountMode(mode);
    var state = {mode: mode, expires: Date.now() + 295000};
    challenge = state;
    var panel = node('deviceChallenge');
    panel.textContent = '';
    panel.appendChild(el('p', 'Preparing sign-in…'));
    panel.appendChild(button('Cancel', function() {
      return cancelChallenge().then(function() { selectAccountMode(''); message(''); });
    }));
    var prefix = '/Bluefin/Accounts/Passkey';
    var initiation = mode === 'passkey'
      ? request(prefix + '/Initiate', 'POST', {})
      : request('/QuickConnect/Initiate', 'POST', {});
    return initiation.then(function(result) {
      if (mode === 'passkey') state.proof = result.proof;
      if (challenge !== state) {
        return state.proof ? request(prefix + '/Cancel', 'POST', state.proof) : undefined;
      }
      panel.firstChild.textContent = mode === 'passkey'
        ? 'Scan with your phone and confirm the code.'
        : 'Enter this code in another client signed in to your server.';
      panel.insertBefore(el('strong', mode === 'passkey' ? result.code : result.Code), panel.lastChild);
      if (mode === 'passkey') {
        var expected = endpoint(prefix + '/QR/');
        if (typeof result.qr_url !== 'string' || !result.qr_url.startsWith(expected) ||
            !/^[a-f0-9]{32}$/.test(result.qr_url.slice(expected.length))) throw new Error('Invalid passkey scan link.');
        var image = el('img');
        image.src = result.qr_url;
        image.alt = 'Scan to sign in using a passkey';
        image.width = image.height = 208;
        panel.insertBefore(image, panel.lastChild);
      }
      function poll() {
        if (challenge !== state) return Promise.resolve();
        if (Date.now() > state.expires) return Promise.reject(new Error('This sign-in request expired.'));
        var exchange = mode === 'passkey'
          ? request(prefix + '/Exchange', 'POST', state.proof)
          : request('/QuickConnect/Connect?secret=' + encodeURIComponent(result.Secret)).then(function(current) {
              return current.Authenticated ? request('/Users/AuthenticateWithQuickConnect', 'POST', {Secret: result.Secret}) : {pending: true};
            });
        return exchange.then(function(response) {
          if (challenge !== state) return;
          if (response.pending) return new Promise(function(resolve) { setTimeout(resolve, 3000); }).then(poll);
          return acceptLogin(response).then(function() { challenge = null; });
        });
      }
      return poll();
    }).catch(function(error) {
      if (challenge === state) {
        return cancelChallenge().then(function() { selectAccountMode(''); throw error; });
      }
    });
  }
  function itemsPath(kind,offset){return '/Items?Recursive=true&IncludeItemTypes='+kind+'&Limit=40&StartIndex='+(offset||0)+'&Fields=Overview,RunTimeTicks,MediaStreams,ExternalUrls&SortBy=SortName&SortOrder=Ascending';}
  var browseHistory = [];
  var playbackOpener = null;
  function pushBrowse() {
    var saved = {page: document.createDocumentFragment(), focus: document.activeElement, scroll: window.scrollY};
    while (content.firstChild) saved.page.appendChild(content.firstChild);
    browseHistory.push(saved);
    if (browseHistory.length > 16) browseHistory.shift();
  }
  function backToBrowse() {
    if (!browseHistory.length) {
      if (viewName !== 'library') return show('library');
      return platform.exit();
    }
    var saved = browseHistory.pop();
    epoch++;
    content.textContent = '';
    content.appendChild(saved.page);
    if (content.contains(saved.focus)) saved.focus.focus();
    else focusFirst(content);
    window.scrollTo(0, saved.scroll);
  }
  function show(name){browseHistory=[];viewName=name;page=0;epoch++;content.textContent='';document.getElementById('tracklist').textContent='';
    if(name==='search'){content.appendChild(el('h2','Search'));var form=el('form',undefined,'searchbox'),input=el('input');input.type='search';input.placeholder='Search your library';input.maxLength=128;input.setAttribute('aria-label','Search your library');var go=button('Search',function(){return search(input.value);},true);go.type='submit';form.appendChild(input);form.appendChild(go);form.onsubmit=function(e){e.preventDefault();guard(function(){return search(input.value);});};content.appendChild(form);content.appendChild(el('div',undefined,'results'));input.focus();return;}
    if(name==='guide')return guide();if(name==='queue')return showQueue();
    var section = {
      library: ['Home', 'Movie,Series'],
      movies: ['Movies', 'Movie'],
      shows: ['TV Shows', 'Series'],
      music: ['Music', 'Audio,MusicAlbum']
    }[name];
    if (!section) return;
    content.appendChild(el('h2',section[0]));
    return loadItems(section[1],0);
  }
  function renderItems(items,host){var grid=el('div',undefined,'grid');items.slice(0,40).forEach(function(item){var tile=button('',function(){return details(item);});tile.className='tile';tile.appendChild(el('small',item.Type||'Library item'));tile.appendChild(el('strong',item.Name||'Untitled'));if(item.ProductionYear)tile.appendChild(el('small',String(item.ProductionYear)));grid.appendChild(tile);});host.appendChild(grid);if(!items.length)host.appendChild(el('p','No items are visible to this account.'));}
  function loadItems(types,offset){var version=epoch;return request(itemsPath(types,offset)).then(function(result){if(version!==epoch)return;var old=content.querySelector('.results');if(old)old.remove();var host=el('div',undefined,'results');renderItems(result.Items||[],host);var actions=el('div',undefined,'actions');if(offset)actions.appendChild(button('Previous page',function(){return loadItems(types,Math.max(0,offset-40));}));if((result.Items||[]).length===40)actions.appendChild(button('Next page',function(){return loadItems(types,offset+40);}));host.appendChild(actions);content.appendChild(host);focusFirst(host);});}
  function search(term){if(!term.trim())return;var version=++epoch;return request(itemsPath('Movie,Series,Episode,Audio,MusicAlbum',0)+'&SearchTerm='+encodeURIComponent(term)).then(function(result){if(version!==epoch)return;var host=content.querySelector('.results');host.textContent='';renderItems(result.Items||[],host);focusFirst(host);});}
  function details(item){epoch++;var version=epoch;return request('/Items/'+encodeURIComponent(item.Id)+'?Fields=Overview,MediaStreams,ExternalUrls,RunTimeTicks').then(function(detail){if(version!==epoch)return;pushBrowse();var host=el('section',undefined,'detail');host.appendChild(el('h1',detail.Name||'Untitled'));if(detail.Overview)host.appendChild(el('p',detail.Overview));var actions=el('div',undefined,'actions');
    if(detail.IsFolder||detail.Type==='Series'||detail.Type==='MusicAlbum'){actions.appendChild(button('Browse',function(){return browse(detail);},true));}
    else {actions.appendChild(button('Play',function(){return play(detail);},true));if(detail.UserData&&detail.UserData.PlaybackPositionTicks)actions.appendChild(button('Resume',function(){return play(detail,detail.UserData.PlaybackPositionTicks);}));if(detail.Type==='Audio')actions.appendChild(button('Add to queue',function(){queue=policy.apply({op:'queue',state:queue,action:'add',item:detail.Id.replace(/-/g,'')});message('Added to this TV queue.');}));}
    (detail.ExternalUrls||[]).slice(0,10).forEach(function(link){try{var handoff=policy.apply({op:'handoff',url:link.Url});actions.appendChild(button('Open '+handoff.provider,function(){return stop().then(function(){return platform.handoff(handoff.url);});}));}catch(e){/* Only exact supported public provider watch URLs get a handoff. */}});
    actions.appendChild(button('Back',backToBrowse));host.appendChild(actions);content.appendChild(host);focusFirst(host);
  });}
  function browse(item) {
    var version = ++epoch;
    return request('/Items?ParentId='+encodeURIComponent(item.Id)+'&Limit=40&Fields=Overview,MediaStreams,ExternalUrls,RunTimeTicks').then(function(result) {
      if (version !== epoch) return;
      pushBrowse();
      content.appendChild(el('h2',item.Name));
      content.appendChild(button('Back',backToBrowse));
      renderItems(result.Items||[],content);
      focusFirst(content);
    });
  }
  function guide(beginning, channelOffset) {
    var halfHour = 30 * 60 * 1000, duration = 4 * halfHour, channelLimit = 24;
    var now = Date.now(), start = beginning === undefined ? Math.floor(now / halfHour) * halfHour : beginning;
    var finish = start + duration, offset = channelOffset || 0, version = ++epoch;
    var timeLabel = new Intl.DateTimeFormat(undefined, {hour:'numeric', minute:'2-digit'});
    function time(value) { return timeLabel.format(new Date(value)); }
    function visit(at, from, focusText) {
      return guide(at, from).then(function() {
        Array.prototype.some.call(content.querySelectorAll('button'), function(control) {
          if (!control.disabled && control.textContent === focusText) { control.focus(); return true; }
          return false;
        });
      });
    }
    content.textContent = '';
    content.appendChild(el('h2', 'Live TV'));
    var controls = el('div', undefined, 'actions guide-controls');
    controls.appendChild(button('Earlier', function() { return visit(start-duration, offset, 'Earlier'); }));
    controls.appendChild(button('Now', function() { return visit(undefined, offset, 'Now'); }));
    controls.appendChild(button('Later', function() { return visit(start+duration, offset, 'Later'); }));
    controls.appendChild(el('span', new Date(start).toLocaleDateString(undefined, {weekday:'long', month:'short', day:'numeric'})));
    content.appendChild(controls);
    var body = el('div', undefined, 'guide-body');
    body.appendChild(el('p', 'Loading guide…'));
    content.appendChild(body);

    return request('/LiveTv/Channels?Limit='+channelLimit+'&StartIndex='+offset).then(function(result) {
      if (version !== epoch) return;
      var channels = (result.Items || []).slice(0, channelLimit);
      if (!channels.length) { body.textContent = 'No live channels.'; return; }
      var query = '&ChannelIds='+encodeURIComponent(channels.map(function(ch) { return ch.Id; }).join(','))+
        '&MaxStartDate='+encodeURIComponent(new Date(finish).toISOString())+
        '&MinEndDate='+encodeURIComponent(new Date(start).toISOString());
      return request('/LiveTv/Programs?Limit=250'+query).catch(function(error) {
        if (error.status !== 404) throw error;
        return {Items:[]};
      }).then(function(schedule) {
        if (version !== epoch) return;
        body.textContent = '';
        var byChannel = Object.create(null);
        (schedule.Items || []).forEach(function(program) {
          var a = Date.parse(program.StartDate), b = Date.parse(program.EndDate);
          if (!program.ChannelId || !(a < b && a < finish && b > start)) return;
          var row = byChannel[program.ChannelId] || (byChannel[program.ChannelId] = []);
          row.push({a:Math.max(a,start), b:Math.min(b,finish), title:program.Name||'Programme'});
        });
        var selection = el('div', undefined, 'guide-selection');
        selection.setAttribute('role', 'status');
        body.appendChild(selection);
        var grid = el('div', undefined, 'guide-scroll');
        var heading = el('div', undefined, 'guide-row guide-heading');
        heading.appendChild(el('strong', 'Channel'));
        var scale = el('div', undefined, 'guide-scale');
        for (var i=0; i<4; i++) scale.appendChild(el('span',time(start+i*halfHour)));
        heading.appendChild(scale);
        grid.appendChild(heading);
        channels.forEach(function(channel) {
          var channelName = [channel.Number, channel.Name].filter(Boolean).join(' · ') || 'Channel';
          var row = el('div', undefined, 'guide-row');
          row.appendChild(el('strong', channelName, 'guide-channel'));
          var timeline = el('div', undefined, 'guide-timeline'), cursor = start;
          function block(from, to, title, onNow, programme) {
            var cell = el('div', undefined, onNow ? 'guide-cell on-now' : 'guide-cell');
            cell.style.flexBasis = ((to-from)*100/duration)+'%';
            var label = cell;
            if (programme && to-from >= 2*60*1000) {
              label = button('', function() {
                selection.textContent = channelName+' · '+title+' · '+time(programme.a)+'–'+time(programme.b);
              });
              label.className = 'guide-cell-button';
              label.setAttribute('aria-label',title+', '+channelName+', '+time(programme.a)+' to '+time(programme.b));
              cell.appendChild(label);
            }
            label.appendChild(el('span', title));
            label.appendChild(el('small', onNow ? 'On now' : time(from)));
            timeline.appendChild(cell);
          }
          (byChannel[channel.Id] || []).sort(function(a,b) { return a.a-b.a || a.b-b.b; }).forEach(function(program) {
            if (program.b <= cursor) return;
            if (cursor < program.a) block(cursor, program.a, '—', false);
            block(Math.max(cursor,program.a), program.b, program.title, program.a <= now && program.b > now, program);
            cursor = program.b;
          });
          if (cursor < finish) block(cursor, finish, cursor === start ? 'No guide data' : '—', false);
          if (start <= now && now < finish) {
            var marker = el('i', undefined, 'guide-now');
            marker.style.left = ((now-start)*100/duration)+'%';
            marker.setAttribute('aria-hidden','true');
            timeline.appendChild(marker);
          }
          row.appendChild(timeline);
          grid.appendChild(row);
        });
        body.appendChild(grid);
        if (offset > 0 || (result.TotalRecordCount || 0) > offset+channels.length) {
          var pages = el('div', undefined, 'actions');
          if (offset > 0) pages.appendChild(button('Previous channels', function() { return visit(start,Math.max(0,offset-channelLimit),'Previous channels'); }));
          if ((result.TotalRecordCount || 0) > offset+channels.length) pages.appendChild(button('Next channels', function() { return visit(start,offset+channelLimit,'Next channels'); }));
          body.appendChild(pages);
        }
      });
    }).catch(function(error) {
      if (version !== epoch) return;
      body.textContent = '';
      body.appendChild(el('p',error.message||'Guide unavailable.'));
      body.appendChild(button('Retry',function() { return visit(start,offset,'Retry'); }));
    });
  }
  function mapBounded(items, transform) { var cursor=0, result=new Array(items.length); function next(){var index=cursor++;if(index>=items.length)return Promise.resolve();return Promise.resolve(transform(items[index],index)).then(function(value){result[index]=value;return next();});}return Promise.all([next(),next(),next(),next()]).then(function(){return result;}); }
  function showQueue(){content.appendChild(el('h2','Music queue'));var actions=el('div',undefined,'actions');actions.appendChild(button('Load account queue',function(){return request('/Bluefin/Music/Queue').then(function(result){if(result.queue.itemIds.length>200)throw new Error('This TV supports queues up to 200 songs.');queue={queue:result.queue.itemIds.map(function(id){return id.replace(/-/g,'');}),cursor:result.queue.cursor||0,positionTicks:0};queueRevision=result.revision;return show('queue');});}));actions.appendChild(button('Save account queue',function(){if(queueRevision===null)throw new Error('Load the account queue first to avoid replacing newer changes.');return request('/Bluefin/Music/Queue','PUT',{protocolVersion:1,expectedRevision:queueRevision,queue:{itemIds:queue.queue,cursor:queue.queue.length?queue.cursor:null,shuffle:false,repeat:'off'}}).then(function(result){queueRevision=result.revision;message('Account queue saved.');});}));actions.appendChild(button('Clear this TV queue',function(){queue=policy.apply({op:'queue',state:queue,action:'clear'});return show('queue');}));content.appendChild(actions);if(!queue.queue.length){content.appendChild(el('p','Add a song from Music.'));focusFirst(content);return;}
    return mapBounded(queue.queue,function(id){return request('/Items/'+id).catch(function(){return null;});}).then(function(items){items.forEach(function(item,index){if(item)content.appendChild(button((index===queue.cursor?'▶ ':'')+(item.Name||'Song'),function(){queue.cursor=index;return play(item);}));});focusFirst(content);});}
  var playbackControlsTimer = 0;
  var controlsVisible = false;
  function setControlsVisible(visible) {
    controlsVisible = visible;
    var bar = document.querySelector('.playbar');
    var player = node('now');
    if (!visible && bar.contains(document.activeElement)) player.focus();
    player.classList.toggle('show-controls', visible);
    bar.setAttribute('aria-hidden', visible ? 'false' : 'true');
    Array.prototype.forEach.call(bar.querySelectorAll('button'), function(control) {
      control.tabIndex = visible ? 0 : -1;
    });
  }
  function showPlaybackControls() {
    if (!lease) return;
    clearTimeout(playbackControlsTimer);
    setControlsVisible(true);
    playbackControlsTimer = setTimeout(function() {
      if (lease && !platform.position().paused && node('tracklist').children.length === 0) {
        setControlsVisible(false);
      }
    }, 4500);
  }
  function resetPlaybackControls() {
    clearTimeout(playbackControlsTimer);
    setControlsVisible(false);
  }
  function play(item,start,selection){if(!leaseAvailable)throw new Error('This server needs the TV playback lease extension to play media.');if(busy)return Promise.reject(new Error('Wait for the current action.'));var kind=item.Type==='Audio'?'audio':'video';if(!platform.canPlay(kind))throw new Error('The platform has not reported support for the safe '+kind+' stream.');busy=true;
    var opener = playbackOpener || document.activeElement;
    return stop().then(function(){playbackOpener = opener; var body=policy.apply({op:'lease',item:item.Id,kind:kind,startTicks:start||0});if(selection){body.playback.audioStreamIndex=selection.audio;body.playback.subtitleStreamIndex=selection.subtitle;}return request('/Bluefin/TV/v1/leases','POST',body);}).then(function(result){grantURL(result.releaseUrl,true);var media=grantURL(result.mediaUrl,false);lease=result;current=item;document.getElementById('playingTitle').textContent=item.Name||'Now playing';document.getElementById('now').hidden=false;document.body.classList.add('playing');return platform.play(media);}).then(function(){showPlaybackControls(); document.getElementById('pause').focus();message('Playing on '+platform.name);return report('/Sessions/Playing');}).catch(function(e){return stop().then(function(){throw e;});}).finally(function(){busy=false;});}
  function report(path,keepalive){if(!lease||!current||!token)return Promise.resolve();var position=platform.position();return request(path,'POST',{ItemId:current.Id,PlaySessionId:lease.playSessionId,PositionTicks:Math.round(position.position*10000000),IsPaused:!!position.paused,CanSeek:position.duration>0,PlayMethod:'Transcode'},keepalive).catch(function(){/* Reporting never elevates playback rights or retries account writes. */});}
  function stop(){resetPlaybackControls();var previous=lease;var stopped=report('/Sessions/Playing/Stopped',true);lease=null;current=null;platform.stop();document.getElementById('now').hidden=true;document.body.classList.remove('playing');document.getElementById('tracklist').textContent='';if(playbackOpener && document.body.contains(playbackOpener))playbackOpener.focus();playbackOpener=null;if(previous)message('Playback stopped.');if(previous){grantURL(previous.releaseUrl,true);return Promise.all([stopped,request(previous.releaseUrl,'DELETE',undefined,true).catch(function(){})]);}return Promise.resolve();}
  function step(action){queue=policy.apply({op:'queue',state:queue,action:action});var id=queue.queue[queue.cursor];if(!id)return stop();return request('/Items/'+id).then(function(item){return play(item);});}
  function seek(delta){var position=platform.position();var target=policy.apply({op:'seek',position:Math.round(position.position*1000),delta:delta*1000,duration:Math.round(position.duration*1000)});platform.seek(target/1000);}
  function tracks(){var host=document.getElementById('tracklist');host.textContent='';var nativeTracks=platform.tracks();nativeTracks.forEach(function(track){host.appendChild(button(track.type+' · '+track.label,function(){platform.selectTrack(track);host.textContent='';document.getElementById('tracks').focus();}));});
    // The safe server output may have a single transcoded track. Original item
    // stream indexes re-negotiate a new grant instead of mutating its fixed URL.
    (current&&current.MediaStreams||[]).filter(function(s){return s.Type==='Audio'||s.Type==='Subtitle';}).slice(0,32).forEach(function(track){host.appendChild(button('Source '+track.Type+' · '+(track.DisplayTitle||track.Language||track.Index),function(){var item=current,position=platform.position().position;return play(item,Math.round(position*10000000),{audio:track.Type==='Audio'?track.Index:null,subtitle:track.Type==='Subtitle'?track.Index:null});}));});
    if(!host.children.length)host.appendChild(el('p','No selectable tracks are exposed by this stream.'));
    host.appendChild(button('Back to player',closeTracks));
    focusFirst(host);
  }
  function closeTracks() {
    node('tracklist').textContent = '';
    node('tracks').focus();
    showPlaybackControls();
  }
  function remoteBack() {
    if (lease) {
      if (node('tracklist').children.length) return closeTracks();
      return stop();
    }
    if (token) return backToBrowse();
    if (accountMode) return cancelChallenge().then(function() { selectAccountMode(''); node('passwordChoice').focus(); });
    if (!node('account').hidden) { node('changeServer').click(); return; }
    return platform.exit();
  }
  function focusFirst(host){var target=host.querySelector('button,input');if(target)target.focus();}
  function move(key){var nodes=Array.prototype.filter.call(document.querySelectorAll('button,input'),function(node){return !node.disabled&&node.getClientRects().length&&getComputedStyle(node).visibility!=='hidden';});var active=document.activeElement,rect=active.getBoundingClientRect(),cx=rect.left+rect.width/2,cy=rect.top+rect.height/2;var best=null,score=Infinity;nodes.forEach(function(node){if(node===active)return;var box=node.getBoundingClientRect(),dx=box.left+box.width/2-cx,dy=box.top+box.height/2-cy;var along=key==='ArrowRight'?dx:key==='ArrowLeft'?-dx:key==='ArrowDown'?dy:-dy,across=key==='ArrowLeft'||key==='ArrowRight'?Math.abs(dy):Math.abs(dx);if(along>5&&along+across*3<score){score=along+across*3;best=node;}});if(best){best.focus();best.scrollIntoView({block:'nearest',inline:'nearest'});}}
  node('connect').onsubmit = connect;
  node('signin').onsubmit = signIn;
  node('passwordChoice').onclick = function() { selectAccountMode('password'); };
  node('signupChoice').onclick = function() { if (choices.allowSignups) selectAccountMode('signup'); };
  node('passkeyChoice').onclick = function() { guard(function() { return beginChallenge('passkey'); }); };
  node('codeChoice').onclick = function() { guard(function() { return beginChallenge('quickConnect'); }); };
  node('changeServer').onclick = function() {
    if (busy) return;
    guard(function() {
      return cancelChallenge().then(function() {
        node('account').hidden = true;
        node('connect').hidden = false;
        node('server').focus();
        message('');
      });
    });
  };
  document.querySelectorAll('[data-view]').forEach(function(node){node.onclick=function(){guard(function(){return show(node.dataset.view);});};});
  document.getElementById('pause').onclick=function(){guard(function(){return platform.pause();});};document.getElementById('stop').onclick=function(){guard(stop);};document.getElementById('rewind').onclick=function(){guard(function(){seek(-10);});};document.getElementById('forward').onclick=function(){guard(function(){seek(30);});};document.getElementById('next').onclick=function(){guard(function(){return step('next');});};document.getElementById('previous').onclick=function(){guard(function(){return step('previous');});};document.getElementById('tracks').onclick=function(){guard(tracks);};
  document.getElementById('signout').onclick=function(){guard(function(){return stop().then(function(){return request('/Sessions/Logout','POST').catch(function(){});}).then(function(){token='';user='';queue={queue:[],cursor:0,positionTicks:0};queueRevision=null;location.reload();});});};
  ['pointerdown', 'pointermove', 'touchstart', 'mousemove'].forEach(function(event) {
    node('now').addEventListener(event, showPlaybackControls, {passive: true});
  });
  document.addEventListener('keydown',function(event){
    if (lease) {
      var wasHidden = !controlsVisible;
      showPlaybackControls();
      if (wasHidden && (event.key === 'Enter' || event.keyCode === 13)) {
        event.preventDefault();
        node('pause').focus();
        return;
      }
    }
    var key=(event.key&&event.key!=='Unidentified'?event.key:null)||({37:'ArrowLeft',38:'ArrowUp',39:'ArrowRight',40:'ArrowDown',13:'Enter',10009:'Back',461:'Back',415:'MediaPlay',19:'MediaPause',413:'MediaStop',412:'MediaRewind',417:'MediaFastForward',10232:'MediaTrackPrevious',10233:'MediaTrackNext',10252:'MediaPlayPause'}[event.keyCode]);if(/^Arrow/.test(key)){if(event.target.tagName==='INPUT'&&(key==='ArrowLeft'||key==='ArrowRight'))return;event.preventDefault();move(key);}else if(key==='Back'||key==='Escape'||key==='Backspace'&&event.target.tagName!=='INPUT'){event.preventDefault();guard(remoteBack);}else if(key==='MediaPlayPause'||key==='MediaPlay'||key==='MediaPause'){event.preventDefault();guard(function(){return platform.pause(key==='MediaPlay'?false:key==='MediaPause'?true:undefined);});}else if(key==='MediaStop'){guard(stop);}else if(key==='MediaRewind'){seek(-10);}else if(key==='MediaFastForward'){seek(30);}else if(key==='MediaTrackNext'){guard(function(){return step('next');});}else if(key==='MediaTrackPrevious'){guard(function(){return step('previous');});}});
  document.addEventListener('visibilitychange',function(){if(document.hidden){resetPlaybackControls();guard(stop);requestControllers.forEach(function(c){c.abort();});}else if(token){message('Playback stopped while Bluefin was in the background. Select Play to resume.');}});
  window.addEventListener('pagehide',function(){guard(stop);token='';});
  setInterval(function(){if(!lease)return;var p=platform.position();document.getElementById('clock').textContent=Math.floor(p.position/60)+':'+('0'+Math.floor(p.position%60)).slice(-2);if(Date.now()-lastReport>10000){lastReport=Date.now();guard(function(){return report('/Sessions/Playing/Progress');});}},1000);
  policy.load().then(function(){platform.init(function(){if(current&&current.Type==='Audio'&&queue.queue.length&&queue.cursor+1<queue.queue.length)guard(function(){return step('next');});else guard(stop);},function(e){guard(stop);showError(e);});message('');document.getElementById('server').focus();}).catch(function(e){document.querySelector('button[type=submit]').disabled=true;showError(e);});
}());
