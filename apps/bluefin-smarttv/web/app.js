(function () {
  'use strict';
  var policy=window.BluefinPolicy, platform=window.BluefinPlatform;
  var content=document.getElementById('content'), status=document.getElementById('status');
  var leaseAvailable=false, catalogController=null, searchState=null, searchFacets=null, homeHistory=null, artURLs=[];
  var searchScopes=[['all','All Media',''],['tvShows','TV Shows','Series,Season,Episode'],['movies','Movies','Movie'],['music','Music','Audio,MusicAlbum,MusicArtist,MusicVideo,Playlist'],['liveTv','Live TV','TvChannel,LiveTvProgram']];
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
  function request(path,method,body,keepalive,signal){
    if(signal&&signal.aborted)return Promise.reject(new Error('Search cancelled.'));
    var accountToken=token,controller=window.AbortController?new AbortController():null, timeout=setTimeout(function(){if(controller)controller.abort();},15000);
    var cancel=function(){if(controller)controller.abort();};if(signal)signal.addEventListener('abort',cancel);
    if(controller)requestControllers.push(controller);
    return fetch(endpoint(path),{method:method||'GET',headers:{'Authorization':header(),'Content-Type':'application/json'},body:body===undefined?undefined:JSON.stringify(body),credentials:'omit',cache:'no-store',redirect:'error',referrerPolicy:'no-referrer',signal:controller?controller.signal:undefined,keepalive:!!keepalive})
      .then(function(r){if(r.status===401){if(token===accountToken)token='';throw new Error('Your session expired. Sign in again.');}if(r.status===404){var missing=new Error('This server does not support this Bluefin feature.');missing.status=404;throw missing;}if(!r.ok){var denied=new Error('Server refused the request ('+r.status+').');denied.status=r.status;throw denied;}if(r.status===204)return null;var length=Number(r.headers.get('content-length'));if(length>2097152)throw new Error('Server response is too large.');return r.text().then(function(text){if(text.length>2097152)throw new Error('Server response is too large.');return text?JSON.parse(text):null;});})
      .finally(function(){if(signal)signal.removeEventListener('abort',cancel);clearTimeout(timeout);if(controller){var i=requestControllers.indexOf(controller);if(i>=0)requestControllers.splice(i,1);}});
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
    if(!/^[a-f0-9-]{32,36}$/i.test(result.User.Id||''))throw new Error('Invalid account identifier.');
    token = result.AccessToken; user = result.User.Id; epoch++; searchFacets=null; homeHistory=null;searchState=null;
    if(catalogController)catalogController.abort();
    return request('/Bluefin/TV/v1').catch(function(error) {
      if (error.status === 404) return null;
      throw error;
    }).then(function(caps) {
      leaseAvailable = !!(caps && caps.version === 1 && caps.mediaLease);
      node('nav').hidden = false;node('utilities').hidden=false;
      message(leaseAvailable ? '' : 'Playback is unavailable on this server.');
      return show('home');
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
  function queryPath(path,values){return path+'?'+values.map(function(pair){return encodeURIComponent(pair[0])+'='+encodeURIComponent(String(pair[1]));}).join('&');}
  function itemsPath(kind,offset){var values=[['UserId',user],['Recursive',true],['Limit',40],['StartIndex',offset||0],['Fields','Overview,RunTimeTicks,PrimaryImageAspectRatio'],['SortBy','SortName'],['SortOrder','Ascending']];if(kind)values.push(['IncludeItemTypes',kind]);return queryPath('/Items',values);}
  function settled(promise){return promise.then(function(value){return {value:value};},function(error){return {error:error};});}
  function rows(value){var list=Array.isArray(value)?value:value&&value.Items;if(!Array.isArray(list)||list.length>250)throw new Error('Invalid library response.');return list.filter(function(item){return item&&/^[a-f0-9-]{32,36}$/i.test(item.Id||'');});}
  function readCatalog(path){return request(path,'GET',undefined,false,catalogController&&catalogController.signal);}
  function cancelCatalog(){if(catalogController)catalogController.abort();catalogController=window.AbortController?new AbortController():null;}
  function homeRecommendations(){
    var generation=epoch,account=token;function current(){if(generation!==epoch||account!==token)throw new Error('Home changed.');}
    return readCatalog('/Bluefin/Home/Recommendations').then(function(value){
      current();if(!value||!Array.isArray(value.recommendations)||value.recommendations.length>12)throw new Error('Invalid Home recommendations.');
      homeHistory=value.searchHistorySupported&&Number.isSafeInteger(value.searchHistoryGeneration)?{generation:value.searchHistoryGeneration}:null;
      return value.recommendations.map(function(row){return {title:String(row.title||'Recommended for you').slice(0,256),reason:row.reason,items:rows(row.items).slice(0,18)};});
    }).catch(function(error){
      current();if(error.status!==404)throw error;homeHistory=null;
      return readCatalog(queryPath('/Movies/Recommendations',[['UserId',user],['CategoryLimit',4],['ItemLimit',12],['Fields','Overview']])).then(function(value){
        if(!Array.isArray(value)||value.length>12)throw new Error('Invalid recommendations.');
        var labels={SimilarToRecentlyPlayed:'Because you watched',SimilarToLikedItem:'Because you like',HasDirectorFromRecentlyPlayed:'More from',HasActorFromRecentlyPlayed:'More with',HasLikedDirector:'From a director you like:',HasLikedActor:'With an actor you like:'};
        return value.filter(function(row){return labels[row.RecommendationType];}).map(function(row){return {title:labels[row.RecommendationType]+' '+String(row.BaselineItemName||'a favorite').slice(0,128),items:rows(row.Items).slice(0,18)};});
      });
    });
  }
  function artwork(item,element,backdrop){
    var kind=backdrop&&item.BackdropImageTags&&item.BackdropImageTags.length?'Backdrop':'Primary';
    if(kind==='Primary'&&!(item.ImageTags&&item.ImageTags.Primary))return;
    var version=epoch,account=token;
    fetch(endpoint('/Items/'+encodeURIComponent(item.Id)+'/Images/'+kind+'?maxWidth='+(backdrop?1200:400)+'&quality=85'),{headers:{Authorization:header()},credentials:'omit',cache:'no-store',redirect:'error',referrerPolicy:'no-referrer'})
      .then(function(response){if(!response.ok||!/^image\/(jpeg|png|webp)(;|$)/i.test(response.headers.get('content-type')||'')||Number(response.headers.get('content-length'))>2097152)throw new Error('Artwork unavailable');return response.blob();})
      .then(function(blob){if(blob.size>2097152||version!==epoch||account!==token)return;var url=URL.createObjectURL(blob);artURLs.push(url);if(artURLs.length>96)URL.revokeObjectURL(artURLs.shift());element.style.backgroundImage='linear-gradient(180deg,transparent,rgba(6,23,39,.92)),url("'+url+'")';})
      .catch(function(){});
  }
  function home(){
    var version=epoch;
    return Promise.all([
      settled(readCatalog('/Users/'+encodeURIComponent(user)+'/Items/Resume?Limit=12&Fields=Overview').then(rows)),
      settled(readCatalog('/Shows/NextUp?UserId='+encodeURIComponent(user)+'&Limit=12&Fields=Overview').then(rows)),
      settled(readCatalog('/Users/'+encodeURIComponent(user)+'/Items/Latest?Limit=18&Fields=Overview').then(rows)),
      settled(homeRecommendations())]).then(function(results){
      if(version!==epoch)return;var resume=results[0].value||[],next=results[1].value||[],latest=results[2].value||[],recommended=results[3].value||[];
      var featured=resume[0]||(recommended.filter(function(s){return s.items.length;})[0]||{items:[]}).items[0]||next[0]||latest[0];
      content.appendChild(el('h2','Home'));
      if(featured){var hero=el('section',undefined,'hero');hero.appendChild(el('small','FEATURED'));hero.appendChild(el('h1',featured.Name||'Untitled'));
        var actions=el('div',undefined,'actions');if(['Movie','Episode','Audio'].indexOf(featured.Type)>=0)actions.appendChild(button(featured.UserData&&featured.UserData.PlaybackPositionTicks?'Resume':'Play',function(){return play(featured,featured.UserData&&featured.UserData.PlaybackPositionTicks||0);},true));
        actions.appendChild(button('Details',function(){return details(featured);}));hero.appendChild(actions);artwork(featured,hero,true);content.appendChild(hero);}
      var shelves=[{title:'Continue watching',items:resume},{title:'Up next',items:next}].concat(recommended,[{title:'Recently added',items:latest}]);
      shelves.forEach(function(shelf){if(shelf.items.length){var section=el('section',undefined,'shelf');section.appendChild(el('h2',shelf.title));renderItems(shelf.items,section);content.appendChild(section);}});
      if(!featured)content.appendChild(el('p','Open Collections to browse the titles available to your account.'));
      if(results.some(function(result){return result.error;}))message('Some Home sections could not refresh. Your available account media is shown.');
      focusFirst(content);
    });
  }
  function cleanFilters(value){value=value||{};return {scope:searchScopes.some(function(scope){return scope[0]===value.scope;})?value.scope:'all',
    studioID:/^[a-f0-9-]{32,36}$/i.test(value.studioID||'')?value.studioID:'',tag:typeof value.tag==='string'?value.tag.slice(0,512):'',
    collectionID:/^[a-f0-9-]{32,36}$/i.test(value.collectionID||'')?value.collectionID:''};}
  function hasFacets(filters){return !!(filters.studioID||filters.tag||filters.collectionID);}
  function searchCall(lane,state,start){
    var filters=state.filters,values=[['UserId',user],['Limit',40],['StartIndex',start||0]],path;
    if(state.term)values.push(['SearchTerm',state.term]);
    if(lane==='library'){path='/Items';values.push(['Recursive',true],['Fields','Overview,RunTimeTicks,PrimaryImageAspectRatio']);
      var type=searchScopes.filter(function(scope){return scope[0]===filters.scope;})[0][2];if(type)values.push(['IncludeItemTypes',type]);
      if(filters.studioID)values.push(['StudioIds',filters.studioID]);if(filters.tag)values.push(['Tags',filters.tag]);if(filters.collectionID)values.push(['ParentId',filters.collectionID]);
    }else if(lane==='artists'){path='/Artists';values.push(['Recursive',true]);}
    else if(lane==='programmes'){path='/LiveTv/Programs';values.push(['MinEndDate',state.after]);}
    else path='/LiveTv/Channels';
    return queryPath(path,values);
  }
  function appendPage(page,value){
    var found=rows(value),seen=new Set(page.items.map(function(item){return item.Id.replace(/-/g,'').toLowerCase();}));
    found.forEach(function(item){var key=item.Id.replace(/-/g,'').toLowerCase();if(!seen.has(key)){seen.add(key);page.items.push(item);}});
    page.next+=(Array.isArray(value)?value:value.Items).length;page.total=found.length?Math.max(page.next,Number(value.TotalRecordCount)||page.next):page.next;
  }
  function renderSearch(){
    var host=content.querySelector('.results');if(!host||!searchState)return;host.textContent='';
    searchState.pages.forEach(function(page){if(page.items.length){host.appendChild(el('h2',page.title));renderItems(page.items,host);}
      if(page.next<page.total){var more=button(page.loading?'Loading…':'More '+page.title.toLowerCase(),function(){return searchMore(page.lane);});more.disabled=page.loading;host.appendChild(more);}});
    searchState.problems.forEach(function(problem){host.appendChild(el('p',problem));});
    if(!searchState.pages.some(function(page){return page.items.length||page.loading;}))host.appendChild(el('p','No matching media for this account.'));
  }
  function search(term,filters){
    if(viewName!=='search')return;cancelCatalog();var version=++epoch;
    var selected=cleanFilters(filters||searchState&&searchState.filters),lanes=hasFacets(selected)?['library']:selected.scope==='liveTv'?['programmes','channels']:['library'];
    if(!hasFacets(selected)&&['all','music'].indexOf(selected.scope)>=0)lanes.push('artists');if(!hasFacets(selected)&&selected.scope==='all')lanes=lanes.concat(['programmes','channels']);
    var names={library:'Your library',artists:'Artists',programmes:'On now and upcoming',channels:'Channels'};
    var own=searchState={term:String(term||'').trim().slice(0,128),filters:selected,after:new Date().toISOString(),problems:[],pages:lanes.map(function(lane){return {lane:lane,title:names[lane],items:[],next:0,total:0,loading:true};})};
    renderSearch();
    if(!own.term&&selected.scope==='all'&&!hasFacets(selected)){own.pages=[];renderSearch();return Promise.resolve();}
    return Promise.all(own.pages.map(function(page){return readCatalog(searchCall(page.lane,own,0)).then(function(value){if(version===epoch&&own===searchState)appendPage(page,value);})
      .catch(function(error){if(version===epoch)own.problems.push(page.title+': '+error.message);}).finally(function(){page.loading=false;});}))
      .then(function(){if(version===epoch)renderSearch();});
  }
  function searchMore(lane){
    var own=searchState,page=own&&own.pages.filter(function(value){return value.lane===lane;})[0],version=epoch;
    if(!page||page.loading||page.next>=page.total)return Promise.resolve();page.loading=true;renderSearch();
    return readCatalog(searchCall(lane,own,page.next)).then(function(value){if(version===epoch&&own===searchState)appendPage(page,value);})
      .catch(function(error){if(version===epoch)own.problems.push(error.message);}).finally(function(){page.loading=false;if(version===epoch)renderSearch();});
  }
  function searchChoice(title,key,options,selected,onChange){
    var label=el('label',title),control=el('select');control.setAttribute('aria-label',title);control.id='filter-'+key;
    options.forEach(function(option){var item=el('option',option[1]);item.value=option[0];control.appendChild(item);});control.value=selected||'';control.onchange=function(){onChange(control.value);};label.appendChild(control);return label;
  }
  function searchScreen(initial){
    var filters=cleanFilters({scope:initial}),form=el('form',undefined,'searchbox'),input=el('input');input.type='search';input.placeholder='Search all media';input.maxLength=128;input.setAttribute('aria-label','Search all media');
    var go=el('button','Search','primary');go.type='submit';form.appendChild(input);form.appendChild(go);form.onsubmit=function(event){event.preventDefault();guard(function(){return search(input.value,filters);});};
    content.appendChild(el('h2','Search'));content.appendChild(form);
    var controls=el('div',undefined,'filters');function changed(key,value){filters[key]=value;guard(function(){return search(input.value,filters);});}
    controls.appendChild(searchChoice('Media type','scope',searchScopes.map(function(scope){return [scope[0],scope[1]];}),filters.scope,function(value){changed('scope',value);}));
    content.appendChild(controls);content.appendChild(el('p','Refinements use your server’s library metadata. Tags can identify services or custom groups.','muted'));content.appendChild(el('div',undefined,'results'));
    var accountToken=token,accountUser=user;
    function currentFacets(){return accountToken===token&&accountUser===user&&viewName==='search'&&content.contains(controls);}
    var loading=searchFacets?Promise.resolve(searchFacets):Promise.all([
      settled(request('/Studios?UserId='+encodeURIComponent(user)+'&Limit=200')),
      settled(request('/Items/Filters2?UserId='+encodeURIComponent(user))),
      settled(request('/Items?UserId='+encodeURIComponent(user)+'&Recursive=true&IncludeItemTypes=BoxSet&Limit=200'))]).then(function(results){
        if(!currentFacets())return null;var facets={studios:[],tags:[],collections:[],truncated:false};
        if(results[0].value){facets.studios=rows(results[0].value).map(function(item){return [item.Id,item.Name];});facets.truncated=Number(results[0].value.TotalRecordCount)>facets.studios.length;}
        if(results[1].value&&Array.isArray(results[1].value.Tags)){facets.tags=results[1].value.Tags.filter(function(tag){return typeof tag==='string';}).slice(0,512).map(function(tag){return [tag,tag];});facets.truncated=facets.truncated||results[1].value.Tags.length>512;}
        if(results[2].value){facets.collections=rows(results[2].value).map(function(item){return [item.Id,item.Name];});facets.truncated=facets.truncated||Number(results[2].value.TotalRecordCount)>facets.collections.length;}
        searchFacets=facets;return facets;
      });
    loading.then(function(facets){if(!facets||!currentFacets())return;
      controls.appendChild(searchChoice('Studio','studio',[['','All studios']].concat(facets.studios),'',function(value){changed('studioID',value);}));
      controls.appendChild(searchChoice('Tag','tag',[['','All tags']].concat(facets.tags),'',function(value){changed('tag',value);}));
      controls.appendChild(searchChoice('Collection','collection',[['','All collections']].concat(facets.collections),'',function(value){changed('collectionID',value);}));
      if(facets.truncated)content.appendChild(el('p','More server groups are available through Collections.','muted'));
    }).catch(function(error){if(currentFacets())message(error.message);});
    input.focus();return search('',filters);
  }
  function collections(){
    var version=epoch;content.appendChild(el('h2','Collections'));
    return readCatalog('/Users/'+encodeURIComponent(user)+'/Views').then(function(result){if(version!==epoch)return;renderItems(rows(result),content);focusFirst(content);});
  }
  function sports(){
    var version=epoch;content.appendChild(el('h2','Sports'));
    return readCatalog('/sports/api/v1/home').then(function(feed){if(version!==epoch)return;
      if(!feed||feed.schema!==1||!Array.isArray(feed.games)||!Array.isArray(feed.local_broadcasts)||feed.presentation_only!==true)throw new Error('Invalid sports feed.');
      feed.games.concat(feed.local_broadcasts).slice(0,200).forEach(function(game){var event=game.event||{},card=el('section',undefined,'sports-card');card.appendChild(el('h3',event.name||game.title||game.native_title||'Sports broadcast'));
        (event.competitors||[]).slice(0,4).forEach(function(team){card.appendChild(el('p',(team.name||team.abbreviation||'')+' '+(team.score||'')));});
        if(event.detail)card.appendChild(el('p',event.detail));if(/^[a-f0-9-]{32,36}$/i.test(game.channel_id||''))card.appendChild(button(game.channel_name||'Open channel',function(){return details({Id:game.channel_id,Type:'TvChannel'});}));content.appendChild(card);});
      if(!feed.games.length&&!feed.local_broadcasts.length)content.appendChild(el('p','Your server has no selected sports events right now.'));
    }).catch(function(error){if(error.status!==404)throw error;return readCatalog('/LiveTv/Programs?UserId='+encodeURIComponent(user)+'&IsSports=true&Limit=40&MinEndDate='+encodeURIComponent(new Date().toISOString())).then(function(value){if(version===epoch)renderItems(rows(value),content);});});
  }
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
      if (viewName !== 'home') return show('home');
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
  function show(name){
    var previous=viewName;cancelCatalog();browseHistory=[];viewName=name==='library'?'home':name;page=0;epoch++;searchState=null;content.textContent='';document.getElementById('tracklist').textContent='';
    Array.prototype.forEach.call(document.querySelectorAll('#nav [data-view]'),function(control){control.setAttribute('aria-current',control.dataset.view===viewName?'page':'false');});
    if(viewName==='search')return searchScreen('all');
    if(viewName==='guide')return guide();if(viewName==='queue')return showQueue();if(viewName==='home')return home();if(viewName==='sports')return sports();if(viewName==='collections')return collections();
    if(viewName==='account'){content.appendChild(el('h2','Account'));content.appendChild(button('Music queue',function(){return show('queue');}));content.appendChild(button('Sign out',signOut));focusFirst(content);return Promise.resolve();}
    var section={movies:['Movies','Movie'],shows:['TV Shows','Series'],music:['Music','Audio,MusicAlbum,MusicArtist,Playlist']}[viewName];
    if(!section)return Promise.resolve();content.appendChild(el('h2',section[0]));return loadItems(section[1],0);
  }
  function renderItems(items,host){
    var grid=el('div',undefined,'grid');items.slice(0,200).forEach(function(item){
      var tile=button('',function(){return details(item);});tile.className='tile poster';var copy=el('span',undefined,'poster-copy');
      copy.appendChild(el('strong',item.Name||'Untitled'));if(item.ProductionYear)copy.appendChild(el('small',String(item.ProductionYear)));
      var position=item.UserData&&Number(item.UserData.PlaybackPositionTicks),duration=Number(item.RunTimeTicks);
      if(position>0&&duration>0){var progress=el('progress');progress.max=duration;progress.value=Math.min(duration,position);progress.setAttribute('aria-label','Watched');copy.appendChild(progress);}
      tile.appendChild(copy);artwork(item,tile,false);grid.appendChild(tile);
    });host.appendChild(grid);if(!items.length)host.appendChild(el('p','No items are visible to this account.'));
  }
  function loadItems(types,offset){var version=epoch;return readCatalog(itemsPath(types,offset)).then(function(result){if(version!==epoch)return;var old=content.querySelector('.results');if(old)old.remove();var host=el('div',undefined,'results');renderItems(result.Items||[],host);var actions=el('div',undefined,'actions');if(offset)actions.appendChild(button('Previous page',function(){return loadItems(types,Math.max(0,offset-40));}));if((result.Items||[]).length===40)actions.appendChild(button('Next page',function(){return loadItems(types,offset+40);}));host.appendChild(actions);content.appendChild(host);focusFirst(host);});}
  function details(item){if((item.Type==='Program'||item.Type==='LiveTvProgram')&&/^[a-f0-9-]{32,36}$/i.test(item.ChannelId||''))item={Id:item.ChannelId,Type:'TvChannel'};epoch++;var version=epoch;return request('/Items/'+encodeURIComponent(item.Id)+'?Fields=Overview,MediaStreams,ExternalUrls,RunTimeTicks').then(function(detail){if(version!==epoch)return;pushBrowse();var host=el('section',undefined,'detail');host.appendChild(el('h1',detail.Name||'Untitled'));if(detail.Overview)host.appendChild(el('p',detail.Overview));var actions=el('div',undefined,'actions');
    if(detail.IsFolder||['Series','Season','MusicAlbum','MusicArtist','Playlist','BoxSet','CollectionFolder'].indexOf(detail.Type)>=0){actions.appendChild(button('Browse',function(){return browse(detail);},true));}
    else {actions.appendChild(button('Play',function(){return play(detail);},true));if(detail.UserData&&detail.UserData.PlaybackPositionTicks)actions.appendChild(button('Resume',function(){return play(detail,detail.UserData.PlaybackPositionTicks);}));if(detail.Type==='Audio')actions.appendChild(button('Add to queue',function(){queue=policy.apply({op:'queue',state:queue,action:'add',item:detail.Id.replace(/-/g,'')});message('Added to this TV queue.');}));}
    (detail.ExternalUrls||[]).slice(0,10).forEach(function(link){try{var handoff=policy.apply({op:'handoff',url:link.Url});var label=(handoff.kind==='player'?'Watch on ':'Open on ')+handoff.provider;actions.appendChild(button(label,function(){return stop().then(function(){return platform.handoff(handoff.url);});}));}catch(e){/* Only exact supported public provider watch URLs get a handoff. */}});
    actions.appendChild(button('Back',backToBrowse));host.appendChild(actions);content.appendChild(host);focusFirst(host);
  });}
  function browse(item,offset) {
    offset=offset||0;var version = ++epoch;
    var filter=item.Type==='MusicArtist'?'ArtistIds='+encodeURIComponent(item.Id)+'&IncludeItemTypes=MusicAlbum,Audio,MusicVideo&Recursive=true':'ParentId='+encodeURIComponent(item.Id);
    return readCatalog('/Items?UserId='+encodeURIComponent(user)+'&'+filter+'&Limit=40&StartIndex='+offset+'&Fields=Overview,MediaStreams,ExternalUrls,RunTimeTicks').then(function(result) {
      if (version !== epoch) return;
      pushBrowse();
      content.appendChild(el('h2',item.Name));
      content.appendChild(button('Back',backToBrowse));
      renderItems(result.Items||[],content);
      if(offset)content.appendChild(button('Previous page',function(){return browse(item,Math.max(0,offset-40));}));
      if((result.Items||[]).length===40)content.appendChild(button('Next page',function(){return browse(item,offset+40);}));
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
  function focusFirst(host){var target=host.querySelector('button,input,select');if(target)target.focus();}
  function move(key){var nodes=Array.prototype.filter.call(document.querySelectorAll('button,input,select'),function(node){return !node.disabled&&node.getClientRects().length&&getComputedStyle(node).visibility!=='hidden';});var active=document.activeElement,rect=active.getBoundingClientRect(),cx=rect.left+rect.width/2,cy=rect.top+rect.height/2;var best=null,score=Infinity;nodes.forEach(function(node){if(node===active)return;var box=node.getBoundingClientRect(),dx=box.left+box.width/2-cx,dy=box.top+box.height/2-cy;var along=key==='ArrowRight'?dx:key==='ArrowLeft'?-dx:key==='ArrowDown'?dy:-dy,across=key==='ArrowLeft'||key==='ArrowRight'?Math.abs(dy):Math.abs(dx);if(along>5&&along+across*3<score){score=along+across*3;best=node;}});if(best){best.focus();best.scrollIntoView({block:'nearest',inline:'nearest'});}}
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
  function signOut(){return stop().then(function(){return request('/Sessions/Logout','POST').catch(function(){});}).then(function(){
    if(catalogController)catalogController.abort();epoch++;token='';user='';searchState=null;searchFacets=null;homeHistory=null;
    artURLs.forEach(function(url){URL.revokeObjectURL(url);});artURLs=[];queue={queue:[],cursor:0,positionTicks:0};queueRevision=null;location.reload();
  });}

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
    var key=(event.key&&event.key!=='Unidentified'?event.key:null)||({37:'ArrowLeft',38:'ArrowUp',39:'ArrowRight',40:'ArrowDown',13:'Enter',10009:'Back',461:'Back',415:'MediaPlay',19:'MediaPause',413:'MediaStop',412:'MediaRewind',417:'MediaFastForward',10232:'MediaTrackPrevious',10233:'MediaTrackNext',10252:'MediaPlayPause'}[event.keyCode]);if(/^Arrow/.test(key)){if(event.target.tagName==='SELECT')return;if(event.target.tagName==='INPUT'&&(key==='ArrowLeft'||key==='ArrowRight'))return;event.preventDefault();move(key);}else if(key==='Back'||key==='Escape'||key==='Backspace'&&event.target.tagName!=='INPUT'){event.preventDefault();guard(remoteBack);}else if(key==='MediaPlayPause'||key==='MediaPlay'||key==='MediaPause'){event.preventDefault();guard(function(){return platform.pause(key==='MediaPlay'?false:key==='MediaPause'?true:undefined);});}else if(key==='MediaStop'){guard(stop);}else if(key==='MediaRewind'){seek(-10);}else if(key==='MediaFastForward'){seek(30);}else if(key==='MediaTrackNext'){guard(function(){return step('next');});}else if(key==='MediaTrackPrevious'){guard(function(){return step('previous');});}});
  document.addEventListener('visibilitychange',function(){if(document.hidden){resetPlaybackControls();guard(stop);requestControllers.forEach(function(c){c.abort();});}else if(token){message('Playback stopped while Bluefin was in the background. Select Play to resume.');}});
  window.addEventListener('pagehide',function(){guard(stop);token='';});
  setInterval(function(){if(!lease)return;var p=platform.position();document.getElementById('clock').textContent=Math.floor(p.position/60)+':'+('0'+Math.floor(p.position%60)).slice(-2);if(Date.now()-lastReport>10000){lastReport=Date.now();guard(function(){return report('/Sessions/Playing/Progress');});}},1000);
  policy.load().then(function(){platform.init(function(){if(current&&current.Type==='Audio'&&queue.queue.length&&queue.cursor+1<queue.queue.length)guard(function(){return step('next');});else guard(stop);},function(e){guard(stop);showError(e);});message('');document.getElementById('server').focus();}).catch(function(e){document.querySelector('button[type=submit]').disabled=true;showError(e);});
}());
