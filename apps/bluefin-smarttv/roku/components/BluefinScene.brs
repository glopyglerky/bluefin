' Original native SceneGraph adapter. Roku cannot execute the Rust/WASM policy.
sub init()
    m.top.backgroundColor = "0x061727FF"
    m.list = m.top.findNode("list")
    m.categories = m.top.findNode("categories")
    m.utilities = m.top.findNode("utilities")
    m.categories.ObserveField("itemSelected", "categorySelected")
    m.utilities.ObserveField("itemSelected", "utilitySelected")
    m.accountGeneration = 0
    m.viewGeneration = 0
    m.userId = ""
    m.search = invalid
    m.home = invalid
    m.facetCache = {}
    m.guide = m.top.findNode("guide")
    m.guidePending = false
    m.title = m.top.findNode("title")
    m.status = m.top.findNode("status")
    m.accountQr = m.top.findNode("accountQr")
    m.accountPoll = m.top.findNode("accountPoll")
    m.accountPoll.ObserveField("fire", "pollAccount")
    m.accountOptions = {}
    m.accountMode = ""
    m.pendingSignIn = invalid
    m.serverTitle = "Sign in"
    m.video = m.top.findNode("video")
    m.audio = m.top.findNode("audio")
    m.progress = m.top.findNode("progress")
    m.list.ObserveField("itemSelected", "selected")
    m.guide.ObserveField("programSelected", "guideSelected")
    m.video.ObserveField("state", "playerState")
    m.audio.ObserveField("state", "playerState")
    m.progress.ObserveField("fire", "reportProgress")
    m.token = ""
    m.canLease = false
    m.server = ""
    m.queue = []
    m.cursor = 0
    m.task = invalid
    m.requests = []
    m.lease = invalid
    m.current = invalid
    m.player = m.video
    m.device = CreateObject("roDeviceInfo").GetRandomUUID()
    m.actions = []
    m.history = []
    m.playbackScreen = invalid
    menu()
end sub
sub rows(title as String, labels as Object, actions as Object)
    m.guide.visible = false
    m.accountQr.visible = false
    m.title.visible = true
    m.status.visible = true
    m.list.visible = true
    m.title.text = title
    if m.token <> ""
        m.list.translation = [330,215]
        m.list.itemSize = [880,64]
        m.title.translation = [330,115]
        m.title.width = 880
        m.status.translation = [330,650]
        m.status.width = 880
    else
        m.list.translation = [70,215]
        m.list.itemSize = [1120,64]
        m.title.translation = [70,115]
        m.title.width = 1130
        m.status.translation = [70,650]
        m.status.width = 1130
    end if
    root = CreateObject("roSGNode", "ContentNode")
    for each label in labels
        child = root.CreateChild("ContentNode")
        child.title = label
    end for
    m.actions = actions
    m.list.content = root
    m.list.jumpToItem = 0
    m.list.SetFocus(true)
end sub
function screen() as Object
    return {title:m.title.text, status:m.status.text, content:m.list.content, actions:m.actions, index:m.list.itemFocused, guide:m.guide.visible, base:m.libraryBase, offset:m.libraryOffset, libraryTitle:m.libraryTitle}
end function
sub restoreScreen(previous as Object)
    m.guidePending = false
    m.title.text = previous.title
    m.status.text = previous.status
    m.title.visible = true
    m.status.visible = true
    m.libraryBase = previous.base
    m.libraryOffset = previous.offset
    m.libraryTitle = previous.libraryTitle
    m.guide.visible = previous.guide
    m.list.visible = not previous.guide
    if previous.guide
        m.guide.SetFocus(true)
    else
        m.actions = previous.actions
        m.list.content = previous.content
        if previous.index >= 0 then m.list.jumpToItem = previous.index
        m.list.SetFocus(true)
    end if
end sub
sub rememberScreen()
    if m.history.Count() >= 16 then m.history.Shift()
    m.history.Push(screen())
end sub
function previousScreen() as Boolean
    invalidateCatalog()
    if m.history.Count() = 0 then return false
    restoreScreen(m.history.Pop())
    return true
end function
sub leavePlayer()
    previous = m.playbackScreen
    m.playbackScreen = invalid
    stopPlayer()
    if previous <> invalid then restoreScreen(previous)
end sub
sub menu()
    invalidateCatalog()
    m.history = []
    m.playbackScreen = invalid
    m.guidePending = false
    m.categories.visible = m.token <> ""
    m.utilities.visible = m.token <> ""
    if m.token = ""
        rows("Bluefin", ["Connect to server"], [{kind:"connect"}])
    else
        m.categoryActions = ["home", "sports", "shows", "movies", "guide", "music"]
        m.utilityActions = ["search", "collections", "account"]
        m.categories.content = labelContent(["Home", "Sports", "TV Shows", "Movies", "Live TV", "Music"])
        m.utilities.content = labelContent(["Search", "Collections", "Account"])
        loadHome()
    end if
end sub
function labelContent(labels as Object) as Object
    root = CreateObject("roSGNode", "ContentNode")
    for each label in labels
        child = root.CreateChild("ContentNode")
        child.title = label
    end for
    return root
end function
sub categorySelected()
    index = m.categories.itemSelected
    if index < 0 or index >= m.categoryActions.Count() then return
    invalidateCatalog()
    m.history = []
    activate({kind:m.categoryActions[index]})
end sub
sub utilitySelected()
    index = m.utilities.itemSelected
    if index < 0 or index >= m.utilityActions.Count() then return
    invalidateCatalog()
    activate({kind:m.utilityActions[index]})
end sub
sub accountMenu()
    labels = ["Sign in"]
    actions = [{kind:"passwordSignIn"}]
    if m.accountOptions.passkeys = true
        labels.Push("Sign in with a passkey")
        actions.Push({kind:"passkeySignIn"})
    end if
    if m.accountOptions.quickConnect = true
        labels.Push("Quick Connect")
        actions.Push({kind:"quickConnectSignIn"})
    end if
    if m.accountOptions.allowSignups = true
        labels.Push("Create account")
        actions.Push({kind:"createAccount"})
    end if
    labels.Push("Change server")
    actions.Push({kind:"connect"})
    rows(m.serverTitle, labels, actions)
end sub
sub cancelAccount()
    m.accountPoll.control = "stop"
    active = m.pendingSignIn
    m.pendingSignIn = invalid
    if active <> invalid and active.proof <> invalid
        api("passkeyCancel", "/Bluefin/Accounts/Passkey/Cancel", "POST", active.proof)
    end if
    accountMenu()
end sub
sub pollAccount()
    if m.task <> invalid or m.pendingSignIn = invalid then return
    m.pendingSignIn.polls = m.pendingSignIn.polls + 1
    if m.pendingSignIn.polls > 98
        cancelAccount()
        m.status.text = "This request expired. Start again to sign in."
    else if m.pendingSignIn.proof <> invalid
        api("passkeyExchange", "/Bluefin/Accounts/Passkey/Exchange", "POST", m.pendingSignIn.proof)
    else if m.pendingSignIn.secret <> invalid
        transfer = CreateObject("roUrlTransfer")
        api("quickConnectStatus", "/QuickConnect/Connect?secret=" + transfer.Escape(m.pendingSignIn.secret), "GET", invalid)
    end if
end sub
sub selected()
    index = m.list.itemSelected
    if index < 0 or index >= m.actions.Count() then return
    action = m.actions[index]
    activate(action)
end sub
sub activate(action as Object)
    if action.kind = "connect"
        keyboard("Server HTTPS URL", "server", false)
    else if action.kind = "passwordSignIn" or action.kind = "createAccount"
        m.accountMode = action.kind
        keyboard("Username", "username", false)
    else if action.kind = "passkeySignIn" or action.kind = "quickConnectSignIn"
        m.pendingSignIn = {polls:0}
        rows("Connecting…", ["Cancel"], [{kind:"cancelAccount"}])
        if action.kind = "passkeySignIn"
            api("passkeyStart", "/Bluefin/Accounts/Passkey/Initiate", "POST", {})
        else
            api("quickConnectStart", "/QuickConnect/Initiate", "POST", {})
        end if
    else if action.kind = "cancelAccount"
        cancelAccount()
    else if action.kind = "waitAccount"
        return
    else if action.kind = "home"
        invalidateCatalog()
        loadHome()
    else if action.kind = "sports"
        rememberScreen()
        rows("Sports", ["Loading your sports"], [{kind:"waitAccount"}])
        api("sports", "/sports/api/v1/home", "GET", invalid)
    else if action.kind = "collections"
        rememberScreen()
        m.libraryTitle = "Collections"
        loadLibrary("/Users/" + m.userId + "/Views?Limit=100", 0)
    else if action.kind = "account"
        rememberScreen()
        rows("Account", ["Music queue", "Sign out", "Back"], [{kind:"queue"}, {kind:"signout"}, {kind:"back"}])
    else if action.kind = "searchTerm"
        keyboard("Search all media", "search", false)
    else if action.kind = "searchScope"
        rows("Media type", ["All Media", "TV Shows", "Movies", "Music", "Live TV"], [{kind:"setScope",value:"all"},{kind:"setScope",value:"tvShows"},{kind:"setScope",value:"movies"},{kind:"setScope",value:"music"},{kind:"setScope",value:"liveTv"}])
    else if action.kind = "setScope"
        m.search.scope = action.value
        beginSearch()
    else if action.kind = "searchFacet"
        showFacet(action.field)
    else if action.kind = "setFacet"
        m.search[action.field] = action.value
        if action.field = "studioId" then m.search.studioName = action.name
        if action.field = "collectionId" then m.search.collectionName = action.name
        beginSearch()
    else if action.kind = "clearFacets"
        m.search.studioId = ""
        m.search.studioName = "All studios"
        m.search.tag = ""
        m.search.collectionId = ""
        m.search.collectionName = "All collections"
        beginSearch()
    else if action.kind = "searchMore"
        requestSearchPage(action.lane)
    else if action.kind = "movies"
        rememberScreen()
        m.libraryTitle = "Movies"
        loadLibrary("/Items?Recursive=true&IncludeItemTypes=Movie&SortBy=SortName&Limit=100&Fields=Overview,MediaStreams",0)
    else if action.kind = "shows"
        rememberScreen()
        m.libraryTitle = "TV Shows"
        loadLibrary("/Items?Recursive=true&IncludeItemTypes=Series&SortBy=SortName&Limit=100&Fields=Overview,MediaStreams",0)
    else if action.kind = "music"
        rememberScreen()
        m.libraryTitle = "Music"
        loadLibrary("/Items?Recursive=true&IncludeItemTypes=Audio,MusicAlbum,MusicArtist,Playlist&Limit=100&Fields=Overview,MediaStreams",0)
    else if action.kind = "search"
        rememberScreen()
        m.search = {term:"", scope:"all", studioId:"", studioName:"All studios", tag:"", collectionId:"", collectionName:"All collections", pages:[], after:""}
        searchMenu()
    else if action.kind = "guide"
        rememberScreen()
        m.guidePending = true
        m.status.text = "Loading guide…"
        api("guideChannels", "/LiveTv/Channels?Limit=40", "GET", invalid)
    else if action.kind = "queue"
        rememberScreen()
        labels = []
        actions = []
        for each item in m.queue
            labels.Push(item.Name)
            actions.Push({kind:"play", item:item})
        end for
        if labels.Count() = 0
            labels.Push("Add songs from Music")
            actions.Push({kind:"menu"})
        end if
        rows("Music queue on this TV", labels, actions)
    else if action.kind = "page"
        loadLibrary(m.libraryBase,action.offset)
    else if action.kind = "detail"
        invalidateCatalog()
        if action.item.Type = "Program" or action.item.Type = "LiveTvProgram"
            if action.item.ChannelId = invalid
                m.status.text = "This programme has no available channel."
                return
            end if
            action.item = {Id:action.item.ChannelId, Type:"TvChannel"}
        end if
        rememberScreen()
        m.detail = action.item
        api("details", "/Items/" + action.item.Id + "?Fields=Overview,MediaStreams,ExternalUrls,RunTimeTicks", "GET", invalid)
    else if action.kind = "browse"
        rememberScreen()
        m.libraryTitle = action.item.Name
        loadLibrary("/Items?Limit=100&ParentId=" + action.item.Id + "&Fields=Overview,MediaStreams",0)
    else if action.kind = "play"
        m.playbackScreen = screen()
        beginPlay(action.item, invalid)
    else if action.kind = "add"
        if m.queue.Count() < 200
            m.queue.Push(action.item)
            m.status.text = "Added to this TV queue."
        end if
    else if action.kind = "signout"
        invalidateCatalog()
        m.accountGeneration = m.accountGeneration + 1
        m.search = invalid
        m.home = invalid
        m.facetCache = {}
        stopPlayer()
        api("logout", "/Sessions/Logout", "POST", {})
        m.token = ""
        m.queue = []
        menu()
    else if action.kind = "stop"
        leavePlayer()
    else if action.kind = "back"
        if not previousScreen() then menu()
    else if action.kind = "pause"
        if m.player.state = "playing"
            m.player.control = "pause"
        else
            m.player.control = "resume"
        end if
    else if action.kind = "seek"
        target = m.player.position + action.delta
        if target < 0 then target = 0
        if m.player.duration > 0 and target > m.player.duration then target = m.player.duration
        m.player.seek = target
    else if action.kind = "tracks"
        showTracks()
    else if action.kind = "track"
        item = m.current
        action.startTicks = m.player.position * 10000000
        stopPlayer()
        beginPlay(item, action)
    else if action.kind = "controls"
        playbackControls()
    else if action.kind = "provider"
        dialog = CreateObject("roSGNode", "Dialog")
        dialog.title = "Open in the provider app"
        dialog.message = "Open this public link on another device: " + action.url
        dialog.buttons = ["Close"]
        dialog.ObserveField("buttonSelected", "closeProvider")
        m.top.dialog = dialog
    else if action.kind = "next"
        if m.queue.Count() > 0
            m.cursor = (m.cursor + 1) mod m.queue.Count()
            item = m.queue[m.cursor]
            stopPlayer()
            beginPlay(item, invalid)
        end if
    else
        menu()
    end if
end sub
sub keyboard(title as String, kind as String, secret as Boolean)
    m.inputKind = kind
    dialog = CreateObject("roSGNode", "KeyboardDialog")
    dialog.title = title
    dialog.buttons = ["Continue", "Cancel"]
    dialog.keyboard.textEditBox.secureMode = secret
    dialog.ObserveField("buttonSelected", "keyboardDone")
    m.top.dialog = dialog
end sub
sub keyboardDone()
    dialog = m.top.dialog
    if dialog.buttonSelected <> 0
        dialog.close = true
        return
    end if
    text = dialog.text.Trim()
    dialog.close = true
    if m.inputKind = "server"
        if Left(text,8) <> "https://" or Instr(1,text,"@") > 0 or Instr(1,text,"?") > 0 or Instr(1,text,"#") > 0
            m.status.text = "Use an HTTPS URL without credentials, query or fragment."
            return
        end if
        if Right(text,1) = "/" then text = Left(text,Len(text)-1)
        if m.task <> invalid then return
        m.server = text
        m.accountOptions = {}
        api("publicInfo", "/System/Info/Public", "GET", invalid)
    else if m.inputKind = "username"
        m.username = text
        keyboard("Password", "password", true)
    else if m.inputKind = "password"
        password = dialog.text
        dialog.text = ""
        if m.accountMode = "createAccount"
            if Len(password) < 15
                m.status.text = "Use a password of at least 15 characters."
                return
            end if
            m.createdAccountCredentials = {Username:m.username, Pw:password}
            api("createAccount", "/Bluefin/Accounts/Create", "POST", {username:m.username, password:password})
        else
            api("login", "/Users/AuthenticateByName", "POST", {Username:m.username, Pw:password})
        end if
    else if m.inputKind = "search"
        if m.search = invalid then return
        m.search.term = Left(text,128)
        beginSearch()
    end if
end sub
function authorization() as String
    header = "MediaBrowser Client=" + Chr(34) + "Bluefin Roku" + Chr(34) + ", Device=" + Chr(34) + "Roku" + Chr(34) + ", DeviceId=" + Chr(34) + m.device + Chr(34) + ", Version=" + Chr(34) + "0.1.0" + Chr(34)
    if m.token <> "" then header += ", Token=" + Chr(34) + m.token + Chr(34)
    return header
end function
sub api(kind as String, path as String, method as String, body as Dynamic)
    text = ""
    if body <> invalid then text = FormatJson(body)
    request = {kind:kind, server:m.server, path:path, method:method, body:text, authorization:authorization(), accountGeneration:m.accountGeneration, viewGeneration:m.viewGeneration}
    if m.task <> invalid
        if kind <> "report" and m.requests.Count() < 16 then m.requests.Push(request)
        return
    end if
    startRequest(request)
end sub
sub startRequest(request as Object)
    m.requestKind = request.kind
    m.requestContext = request
    m.task = CreateObject("roSGNode", "ApiTask")
    m.task.request = request
    m.task.ObserveField("result", "response")
    m.task.control = "RUN"
end sub
sub drainRequests()
    while m.task = invalid and m.requests.Count() > 0
        request = m.requests.Shift()
        if not catalogRequest(request.kind) or (request.accountGeneration = m.accountGeneration and request.viewGeneration = m.viewGeneration)
            startRequest(request)
        end if
    end while
end sub
sub response()
    result = m.task.result
    m.task.UnobserveField("result")
    m.task = invalid
    if catalogRequest(m.requestKind)
        if m.requestContext.accountGeneration <> m.accountGeneration or m.requestContext.viewGeneration <> m.viewGeneration
            drainRequests()
            return
        end if
    end if
    if not result.ok
        if result.status = 401 and m.requestContext.accountGeneration = m.accountGeneration
            m.accountGeneration = m.accountGeneration + 1
            m.token = ""
            m.userId = ""
            invalidateCatalog()
            stopPlayer()
            menu()
            drainRequests()
            return
        end if
        if Left(m.requestKind,4) = "home"
            if m.requestKind = "homeRecommendations" and result.status = 404
                api("homeLegacy", "/Movies/Recommendations?UserId=" + m.userId + "&CategoryLimit=4&ItemLimit=12&Fields=Overview", "GET", invalid)
            else
                m.home.problem = true
                finishHomePart()
            end if
            drainRequests()
            return
        else if Left(m.requestKind,6) = "search"
            searchFailed(m.requestKind)
            drainRequests()
            return
        else if m.requestKind = "sports" and result.status = 404
            api("sportsGuide", "/LiveTv/Programs?IsSports=true&Limit=100&UserId=" + m.userId, "GET", invalid)
            drainRequests()
            return
        end if
        if m.requestKind = "accountOptions"
            api("legacyOptions", "/QuickConnect/Enabled", "GET", invalid)
            return
        else if m.requestKind = "legacyOptions"
            accountMenu()
            return
        else if m.requestKind = "createAccount"
            m.createdAccountCredentials = invalid
            api("accountOptions", "/Bluefin/Accounts/Options", "GET", invalid)
            m.status.text = "The account could not be created."
            return
        else if Left(m.requestKind,7) = "passkey" or Left(m.requestKind,12) = "quickConnect"
            if m.requestKind <> "passkeyCancel" then cancelAccount()
            m.status.text = "Sign-in request unavailable. Try again."
            drainRequests()
            return
        end if
        if m.requestKind = "guideChannels" or m.requestKind = "guidePrograms" then m.guidePending = false
        if m.requestKind = "capabilities" and result.status = 404
            m.canLease = false
            menu()
            m.status.text = "Browsing is available. This server needs the TV playback lease extension to play media."
            drainRequests()
            return
        end if
        if result.status = 404
            m.status.text = "This server does not support the requested Bluefin feature."
        else
            m.status.text = "Server refused the request. Check your account and connection."
        end if
        drainRequests()
        return
    end if
    data = result.data
    if Left(m.requestKind,4) = "home"
        acceptHomePart(m.requestKind, data)
        drainRequests()
        return
    else if Left(m.requestKind,6) = "search"
        acceptSearchPage(m.requestKind, data)
        drainRequests()
        return
    else if Left(m.requestKind,5) = "facet"
        acceptFacet(m.requestKind, data)
        drainRequests()
        return
    else if m.requestKind = "sports" or m.requestKind = "sportsGuide"
        acceptSports(data)
        drainRequests()
        return
    end if
    if m.requestKind = "publicInfo"
        if data = invalid or data.ServerName = invalid
            m.status.text = "The address did not return media server information."
            return
        end if
        m.serverTitle = data.ServerName
        api("accountOptions", "/Bluefin/Accounts/Options", "GET", invalid)
    else if m.requestKind = "accountOptions"
        m.accountOptions = data
        accountMenu()
    else if m.requestKind = "legacyOptions"
        m.accountOptions = {quickConnect:data = true}
        accountMenu()
    else if m.requestKind = "createAccount"
        credentials = m.createdAccountCredentials
        m.createdAccountCredentials = invalid
        m.accountMode = "passwordSignIn"
        if credentials <> invalid then api("login", "/Users/AuthenticateByName", "POST", credentials)
    else if m.requestKind = "passkeyStart"
        if m.pendingSignIn = invalid
            api("passkeyCancel", "/Bluefin/Accounts/Passkey/Cancel", "POST", data.proof)
            return
        end if
        m.pendingSignIn.proof = data.proof
        prefix = m.server + "/Bluefin/Accounts/Passkey/QR/"
        ticket = CreateObject("roRegex", "^[a-f0-9]{32}$", "")
        if data.qr_url = invalid or Left(data.qr_url,Len(prefix)) <> prefix or not ticket.IsMatch(Mid(data.qr_url,Len(prefix)+1))
            cancelAccount()
            m.status.text = "Invalid passkey scan link."
            return
        end if
        rows("Scan with your phone", ["Confirm code " + data.code, "Cancel"], [{kind:"waitAccount"}, {kind:"cancelAccount"}])
        m.accountQr.uri = data.qr_url
        m.accountQr.visible = true
        m.accountPoll.control = "start"
    else if m.requestKind = "quickConnectStart"
        if m.pendingSignIn = invalid then return
        m.pendingSignIn.secret = data.Secret
        rows("Quick Connect", ["Enter code " + data.Code + " in a signed-in client", "Cancel"], [{kind:"waitAccount"}, {kind:"cancelAccount"}])
        m.accountPoll.control = "start"
    else if m.requestKind = "quickConnectStatus"
        if m.pendingSignIn <> invalid and data.Authenticated = true
            api("quickConnectExchange", "/Users/AuthenticateWithQuickConnect", "POST", {Secret:m.pendingSignIn.secret})
        end if
    else if m.requestKind = "login" or m.requestKind = "passkeyExchange" or m.requestKind = "quickConnectExchange"
        if data = invalid or data.AccessToken = invalid then return
        if m.requestKind <> "login" and m.pendingSignIn = invalid then return
        m.accountPoll.control = "stop"
        m.pendingSignIn = invalid
        tokenPattern = CreateObject("roRegex", "^[a-zA-Z0-9_-]{1,512}$", "")
        if not tokenPattern.IsMatch(data.AccessToken) then return
        if data.User = invalid or data.User.Id = invalid then return
        validUser = CreateObject("roRegex", "^[a-fA-F0-9-]{32,36}$", "")
        if not validUser.IsMatch(data.User.Id) then return
        m.accountGeneration = m.accountGeneration + 1
        m.userId = data.User.Id
        m.token = data.AccessToken
        api("capabilities", "/Bluefin/TV/v1", "GET", invalid)
    else if m.requestKind = "capabilities"
        m.canLease = data.version = 1 and data.mediaLease = true
        if not m.canLease then m.status.text = "This server needs the TV playback lease extension to play media."
        menu()
    else if m.requestKind = "library"
        labels = []
        actions = []
        if data <> invalid and data.Items <> invalid
            for each item in data.Items
                labels.Push(item.Name)
                actions.Push({kind:"detail", item:item})
            end for
        end if
        if m.libraryOffset > 0
            labels.Push("Previous page")
            actions.Push({kind:"page",offset:m.libraryOffset-100})
        end if
        if data.Items <> invalid and data.Items.Count() = 100
            labels.Push("Next page")
            actions.Push({kind:"page",offset:m.libraryOffset+100})
        end if
        labels.Push("Back")
        actions.Push({kind:"back"})
        title = "Library"
        if m.libraryTitle <> invalid then title = m.libraryTitle
        rows(title, labels, actions)
    else if m.requestKind = "details"
        m.detail = data
        labels = []
        actions = []
        if data.IsFolder = true or data.Type = "Series" or data.Type = "MusicAlbum"
            labels.Push("Browse")
            actions.Push({kind:"browse", item:data})
        else
            labels.Push("Play")
            actions.Push({kind:"play", item:data})
        end if
        if data.Type = "Audio"
            labels.Push("Add to queue")
            actions.Push({kind:"add", item:data})
        end if
        if data.ExternalUrls <> invalid
            pattern = CreateObject("roRegex", "^https://www.youtube.com/watch[?]v=[a-zA-Z0-9_-]{11}$", "")
            for each link in data.ExternalUrls
                if link.Url <> invalid and pattern.IsMatch(link.Url)
                    labels.Push("Public YouTube link")
                    actions.Push({kind:"provider",url:link.Url})
                end if
            end for
        end if
        labels.Push("Back")
        actions.Push({kind:"back"})
        rows(data.Name, labels, actions)
        if data.Overview <> invalid then m.status.text = Left(data.Overview,150)
    else if m.requestKind = "guideChannels"
        if m.guidePending then requestGuide(data)
    else if m.requestKind = "guidePrograms"
        if m.guidePending then showGuide(data)
    else if m.requestKind = "lease"
        route = CreateObject("roRegex", "^/Bluefin/TV/v1/media/[a-f0-9]{64}/[0-9]+$", "")
        releaseRoute = CreateObject("roRegex", "^/Bluefin/TV/v1/leases/[a-f0-9]{64}$", "")
        if not route.IsMatch(data.mediaUrl) or not releaseRoute.IsMatch(data.releaseUrl)
            m.status.text = "Server returned an unsafe playback route."
            return
        end if
        m.lease = data
        node = CreateObject("roSGNode", "ContentNode")
        node.url = m.server + data.mediaUrl
        node.title = m.current.Name
        if m.current.Type = "Audio"
            node.streamFormat = "mp3"
            m.player = m.audio
        else
            node.streamFormat = "hls"
            m.player = m.video
            m.video.visible = true
            m.video.SetFocus(true)
        end if
        m.player.content = node
        m.player.control = "play"
        m.progress.control = "start"
        if m.player = m.video then showNativeVideo() else playbackControls()
        api("report", "/Sessions/Playing", "POST", playbackReport())
    else if m.requestKind = "release"
        if m.pendingItem <> invalid
            item = m.pendingItem
            selection = m.pendingSelection
            m.pendingItem = invalid
            beginPlay(item, selection)
        end if
    end if
    drainRequests()
end sub
sub requestGuide(data as Dynamic)
    if data = invalid or data.Items = invalid or data.Items.Count() = 0
        m.guidePending = false
        rows("Live TV", ["Back"], [{kind:"menu"}])
        m.status.text = "No live channels."
        return
    end if
    m.guideChannels = data.Items
    now = CreateObject("roDateTime").AsSeconds()
    m.guideStart = now - (now mod 1800)
    starts = CreateObject("roDateTime")
    ends = CreateObject("roDateTime")
    starts.FromSeconds(m.guideStart)
    ends.FromSeconds(m.guideStart + 21600)
    ids = ""
    validId = CreateObject("roRegex", "^[a-zA-Z0-9-]{1,64}$", "")
    for each channel in m.guideChannels
        if channel.Id <> invalid and validId.IsMatch(channel.Id)
            if ids <> "" then ids = ids + ","
            ids = ids + channel.Id
        end if
    end for
    if ids = ""
        m.guidePending = false
        m.status.text = "No live channels."
        return
    end if
    fromTime = Left(starts.ToISOString(), 19) + "Z"
    toTime = Left(ends.ToISOString(), 19) + "Z"
    api("guidePrograms", "/LiveTv/Programs?Limit=500&ChannelIds=" + ids + "&MinEndDate=" + fromTime + "&MaxStartDate=" + toTime, "GET", invalid)
end sub

sub showGuide(data as Dynamic)
    m.guidePending = false
    root = CreateObject("roSGNode", "ContentNode")
    schedules = {}
    utcDate = CreateObject("roRegex", "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}([.][0-9]+)?(Z|[+]00:?00)$", "")
    if data <> invalid and data.Items <> invalid
        for each program in data.Items
            if program.ChannelId <> invalid and program.StartDate <> invalid and program.EndDate <> invalid
                if utcDate.IsMatch(program.StartDate) and utcDate.IsMatch(program.EndDate)
                    starts = CreateObject("roDateTime")
                    ends = CreateObject("roDateTime")
                    starts.FromSeconds(0)
                    ends.FromSeconds(0)
                    starts.FromISO8601String(Left(program.StartDate, 19))
                    ends.FromISO8601String(Left(program.EndDate, 19))
                    a = starts.AsSeconds()
                    b = ends.AsSeconds()
                    if a > 0 and b > a and b > m.guideStart
                        if a < m.guideStart then a = m.guideStart
                        if a < m.guideStart + 21600
                            if b > m.guideStart + 21600 then b = m.guideStart + 21600
                            if not schedules.DoesExist(program.ChannelId) then schedules[program.ChannelId] = []
                            schedules[program.ChannelId].Push({starts:a, ends:b, program:program})
                        end if
                    end if
                end if
            end if
        end for
    end if
    for each channel in m.guideChannels
        if channel.Id <> invalid and channel.Name <> invalid
            row = root.CreateChild("ContentNode")
            row.title = channel.Name
            if channel.Number <> invalid then row.title = channel.Number.ToStr() + " · " + channel.Name
            schedule = schedules[channel.Id]
            if schedule <> invalid
                schedule.SortBy("starts")
                previousEnd = m.guideStart
                for each slot in schedule
                    a = slot.starts
                    if a < previousEnd then a = previousEnd
                    if slot.ends > a
                        cell = row.CreateChild("ContentNode")
                        cell.playStart = a
                        cell.playDuration = slot.ends - a
                        if slot.program.Name <> invalid then cell.title = slot.program.Name
                        if slot.program.Overview <> invalid then cell.description = slot.program.Overview
                        previousEnd = slot.ends
                    end if
                end for
            end if
        end if
    end for
    if root.GetChildCount() = 0
        rows("Live TV", ["Back"], [{kind:"menu"}])
        m.status.text = "No live channels."
        return
    end if
    first = CreateObject("roDateTime")
    first.FromSeconds(m.guideStart)
    m.title.text = "Live TV"
    m.status.text = ""
    m.guide.contentStartTime = first
    m.guide.content = root
    m.list.visible = false
    m.guide.visible = true
    m.guide.SetFocus(true)
end sub

sub guideSelected()
    if not m.guide.visible or m.guide.content = invalid then return
    channel = m.guide.channelFocused
    programme = m.guide.programSelected
    if channel < 0 or channel >= m.guide.content.GetChildCount() then return
    row = m.guide.content.GetChild(channel)
    if programme < 0 or programme >= row.GetChildCount() then return
    cell = row.GetChild(programme)
    m.status.text = cell.title
    if cell.description <> "" then m.status.text = Left(cell.description, 160)
end sub

sub beginPlay(item as Object, selection as Dynamic)
    if not m.canLease
        m.status.text = "This server needs the TV playback lease extension to play media."
        return
    end if
    if m.lease <> invalid
        m.pendingItem = item
        m.pendingSelection = selection
        stopPlayer()
        return
    end if
    info = CreateObject("roDeviceInfo")
    kind = "video"
    bitrate = 8000000
    if item.Type = "Audio"
        kind = "audio"
        bitrate = 320000
        supported = info.CanDecodeAudio({codec:"mp3", channels:2})
    else
        supported = info.CanDecodeVideo({codec:"mpeg4 avc", profile:"high", level:"4.1"})
    end if
    if supported = invalid or supported.result <> true
        m.status.text = "This Roku did not report support for the safe playback codec."
        return
    end if
    m.current = item
    options = {kind:kind, maxBitrate:bitrate, startTicks:0}
    if selection <> invalid
        if selection.startTicks <> invalid then options.startTicks = selection.startTicks
        if selection.trackType = "Audio" then options.audioStreamIndex = selection.index
        if selection.trackType = "Subtitle" then options.subtitleStreamIndex = selection.index
    end if
    api("lease", "/Bluefin/TV/v1/leases", "POST", {version:1, itemId:item.Id.Replace("-", "").LCase(), playback:options})
end sub
sub showNativeVideo()
    m.list.visible = false
    m.title.visible = false
    m.status.visible = false
    m.video.visible = true
    m.video.SetFocus(true)
end sub
sub playbackControls()
    if m.player = m.video
        showNativeVideo()
    else
        rows("Now playing: " + m.current.Name, ["Pause / resume", "Back 10 seconds", "Forward 30 seconds", "Tracks", "Next queued song", "Stop"], [{kind:"pause"},{kind:"seek",delta:-10},{kind:"seek",delta:30},{kind:"tracks"},{kind:"next"},{kind:"stop"}])
    end if
end sub
sub showTracks()
    labels = []
    actions = []
    if m.current.MediaStreams <> invalid
        for each track in m.current.MediaStreams
            if track.Type = "Audio" or track.Type = "Subtitle"
                name = track.Type + " " + track.Index.ToStr()
                if track.DisplayTitle <> invalid then name = track.DisplayTitle
                labels.Push(name)
                actions.Push({kind:"track", trackType:track.Type, index:track.Index})
            end if
        end for
    end if
    labels.Push("Back to player")
    actions.Push({kind:"controls"})
    rows("Source tracks", labels, actions)
end sub
function playbackReport() as Object
    if m.lease = invalid or m.current = invalid then return {}
    return {ItemId:m.current.Id, PlaySessionId:m.lease.playSessionId, PositionTicks:m.player.position*10000000, IsPaused:m.player.state="paused", CanSeek:m.player.duration>0, PlayMethod:"Transcode"}
end function
sub reportProgress()
    if m.lease <> invalid and m.task = invalid then api("report", "/Sessions/Playing/Progress", "POST", playbackReport())
end sub
sub stopPlayer()
    m.progress.control = "stop"
    m.player.control = "stop"
    m.video.visible = false
    if m.lease <> invalid
        release = m.lease.releaseUrl
        m.lease = invalid
        api("release", release, "DELETE", {})
    end if
end sub
sub playerState()
    state = m.player.state
    if state = "error"
        leavePlayer()
        m.status.text = "The Roku player rejected this stream."
    else if state = "finished"
        leavePlayer()
    end if
end sub
function onKeyEvent(key as String, press as Boolean) as Boolean
    if not press then return false
    if m.token <> "" and m.lease = invalid
        if key = "left" and m.list.HasFocus()
            m.categories.SetFocus(true)
            return true
        else if key = "right" and (m.categories.HasFocus() or m.utilities.HasFocus())
            m.list.SetFocus(true)
            return true
        else if key = "down" and m.categories.HasFocus() and m.categories.itemFocused = 5
            m.utilities.SetFocus(true)
            return true
        else if key = "up" and m.utilities.HasFocus() and m.utilities.itemFocused = 0
            m.categories.SetFocus(true)
            m.categories.jumpToItem = 5
            return true
        end if
    end if
    if key = "back"
        if m.pendingSignIn <> invalid
            cancelAccount()
            return true
        end if
        if m.lease <> invalid
            if m.title.text = "Source tracks"
                playbackControls()
            else
                leavePlayer()
            end if
            return true
        end if
        if m.token <> "" then return previousScreen()
        if m.accountMode <> ""
            m.accountMode = ""
            accountMenu()
            return true
        end if
    else if key = "options" and m.lease <> invalid and m.player = m.video
        m.video.visible = false
        showTracks()
        return true
    else if key = "play" and m.lease <> invalid
        if m.player.state = "playing"
            m.player.control = "pause"
        else
            m.player.control = "resume"
        end if
        return true
    else if key = "replay" and m.lease <> invalid
        target = m.player.position - 10
        if target < 0 then target = 0
        m.player.seek = target
        return true
    end if
    return false
end function

sub closeProvider()
    m.top.dialog.close = true
end sub

sub loadLibrary(base as String, offset as Integer)
    m.libraryBase = base
    m.libraryOffset = offset
    api("library",base+"&StartIndex="+offset.ToStr(),"GET",invalid)
end sub

function catalogRequest(kind as String) as Boolean
    return kind = "library" or kind = "details" or kind = "sports" or kind = "sportsGuide" or Left(kind,4) = "home" or Left(kind,5) = "facet" or Left(kind,6) = "search" or Left(kind,5) = "guide"
end function
sub invalidateCatalog()
    m.viewGeneration = m.viewGeneration + 1
    retained = []
    for each request in m.requests
        if not catalogRequest(request.kind) then retained.Push(request)
    end for
    m.requests = retained
    if m.task <> invalid and catalogRequest(m.requestKind)
        m.task.UnobserveField("result")
        m.task.control = "STOP"
        m.task = invalid
    end if
    drainRequests()
end sub
function catalogItems(data as Dynamic) as Object
    if data = invalid then return []
    if Type(data) = "roArray" then return data
    if Type(data) <> "roAssociativeArray" then return []
    if data.Items = invalid then return []
    if Type(data.Items) <> "roArray" then return []
    return data.Items
end function
sub loadHome()
    m.home = {resume:[], next:[], latest:[], recommendations:[], remaining:4, problem:false}
    rows("Home", ["Loading your account"], [{kind:"waitAccount"}])
    api("homeResume", "/Users/" + m.userId + "/Items/Resume?Limit=12&Fields=Overview,RunTimeTicks", "GET", invalid)
    api("homeNext", "/Shows/NextUp?UserId=" + m.userId + "&Limit=12&Fields=Overview,RunTimeTicks", "GET", invalid)
    api("homeLatest", "/Users/" + m.userId + "/Items/Latest?Limit=18&Fields=Overview,RunTimeTicks", "GET", invalid)
    api("homeRecommendations", "/Bluefin/Home/Recommendations", "GET", invalid)
end sub
sub acceptHomePart(kind as String, data as Dynamic)
    if m.home = invalid then return
    if kind = "homeResume"
        m.home.resume = catalogItems(data)
    else if kind = "homeNext"
        m.home.next = catalogItems(data)
    else if kind = "homeLatest"
        m.home.latest = catalogItems(data)
    else if kind = "homeRecommendations"
        if data <> invalid and data.recommendations <> invalid
            for each shelf in data.recommendations
                if shelf.items <> invalid and shelf.title <> invalid and m.home.recommendations.Count() < 4
                    m.home.recommendations.Push({title:Left(shelf.title,180), items:catalogItems(shelf.items)})
                end if
            end for
        end if
    else if kind = "homeLegacy"
        labels = {SimilarToRecentlyPlayed:"Because you watched", SimilarToLikedItem:"Because you like", HasDirectorFromRecentlyPlayed:"More from", HasActorFromRecentlyPlayed:"More with", HasLikedDirector:"From a director you like:", HasLikedActor:"With an actor you like:"}
        for each shelf in catalogItems(data)
            prefix = labels[shelf.RecommendationType]
            if prefix <> invalid and shelf.BaselineItemName <> invalid
                m.home.recommendations.Push({title:prefix + " " + Left(shelf.BaselineItemName,128), items:catalogItems(shelf.Items)})
            end if
        end for
    end if
    finishHomePart()
end sub
sub finishHomePart()
    if m.home = invalid then return
    m.home.remaining = m.home.remaining - 1
    if m.home.remaining > 0 then return
    shelves = [{title:"Continue watching",items:m.home.resume},{title:"Up next",items:m.home.next}]
    for each shelf in m.home.recommendations
        shelves.Push(shelf)
    end for
    shelves.Push({title:"Recently added",items:m.home.latest})
    labels = []
    actions = []
    for each shelf in shelves
        if shelf.items.Count() > 0
            labels.Push(shelf.title)
            actions.Push({kind:"waitAccount"})
            count = 0
            for each item in shelf.items
                if item.Id <> invalid and item.Name <> invalid and count < 18
                    labels.Push(item.Name)
                    actions.Push({kind:"detail",item:item})
                    count = count + 1
                end if
            end for
        end if
    end for
    if labels.Count() = 0
        labels.Push("Open Collections to browse your account")
        actions.Push({kind:"collections"})
    end if
    rows("Home", labels, actions)
    if m.home.problem then m.status.text = "Some Home sections could not refresh."
end sub
sub searchMenu()
    if m.search = invalid then return
    scopeNames = {all:"All Media",tvShows:"TV Shows",movies:"Movies",music:"Music",liveTv:"Live TV"}
    term = "Enter search"
    if m.search.term <> "" then term = "Search: " + m.search.term
    tag = "All tags"
    if m.search.tag <> "" then tag = m.search.tag
    labels = [term, "Media type: " + scopeNames[m.search.scope], "Studio: " + m.search.studioName, "Tag: " + tag, "Collection: " + m.search.collectionName, "Clear refinements"]
    actions = [{kind:"searchTerm"},{kind:"searchScope"},{kind:"searchFacet",field:"studioId"},{kind:"searchFacet",field:"tag"},{kind:"searchFacet",field:"collectionId"},{kind:"clearFacets"}]
    for each page in m.search.pages
        if page.items.Count() > 0
            labels.Push(page.title)
            actions.Push({kind:"waitAccount"})
            for each item in page.items
                if item.Name <> invalid and item.Id <> invalid
                    labels.Push(item.Name)
                    actions.Push({kind:"detail",item:item})
                end if
            end for
        end if
        if page.next < page.total and not page.busy
            labels.Push("More " + page.title.LCase())
            actions.Push({kind:"searchMore",lane:page.lane})
        end if
    end for
    rows("Search",labels,actions)
end sub
function scopeTypes(scope as String) as String
    if scope = "tvShows" then return "Series,Season,Episode"
    if scope = "movies" then return "Movie"
    if scope = "music" then return "Audio,MusicAlbum,MusicArtist,MusicVideo,Playlist"
    if scope = "liveTv" then return "TvChannel,LiveTvProgram"
    return ""
end function
sub beginSearch()
    invalidateCatalog()
    if m.search = invalid then return
    now = CreateObject("roDateTime")
    m.search.after = Left(now.ToISOString(),19) + "Z"
    facets = m.search.studioId <> "" or m.search.tag <> "" or m.search.collectionId <> ""
    lanes = ["library"]
    if not facets
        if m.search.scope = "liveTv"
            lanes = ["programmes","channels"]
        else
            if m.search.scope = "all" or m.search.scope = "music" then lanes.Push("artists")
            if m.search.scope = "all"
                lanes.Push("programmes")
                lanes.Push("channels")
            end if
        end if
    end if
    m.search.pages = []
    titles = {library:"Your library",artists:"Artists",programmes:"On now and upcoming",channels:"Channels"}
    for each lane in lanes
        m.search.pages.Push({lane:lane,title:titles[lane],items:[],next:0,total:0,busy:false})
    end for
    searchMenu()
    if m.search.term = "" and m.search.scope = "all" and not facets then return
    m.status.text = "Searching your account..."
    for each lane in lanes
        requestSearchPage(lane)
    end for
end sub
function searchPage(lane as String) as Dynamic
    if m.search = invalid then return invalid
    for each page in m.search.pages
        if page.lane = lane then return page
    end for
    return invalid
end function
function searchPath(lane as String, start as Integer) as String
    transfer = CreateObject("roUrlTransfer")
    query = "?UserId=" + transfer.Escape(m.userId) + "&Limit=60&StartIndex=" + start.ToStr()
    if m.search.term <> "" then query = query + "&SearchTerm=" + transfer.Escape(m.search.term)
    if lane = "library"
        query = query + "&Recursive=true&Fields=Overview,RunTimeTicks"
        types = scopeTypes(m.search.scope)
        if types <> "" then query = query + "&IncludeItemTypes=" + transfer.Escape(types)
        if m.search.studioId <> "" then query = query + "&StudioIds=" + transfer.Escape(m.search.studioId)
        if m.search.tag <> "" then query = query + "&Tags=" + transfer.Escape(m.search.tag)
        if m.search.collectionId <> "" then query = query + "&ParentId=" + transfer.Escape(m.search.collectionId)
        return "/Items" + query
    else if lane = "artists"
        return "/Artists" + query + "&Recursive=true"
    else if lane = "programmes"
        return "/LiveTv/Programs" + query + "&MinEndDate=" + transfer.Escape(m.search.after)
    end if
    return "/LiveTv/Channels" + query
end function
sub requestSearchPage(lane as String)
    page = searchPage(lane)
    if page = invalid or page.busy then return
    page.busy = true
    api("search" + lane, searchPath(lane,page.next), "GET", invalid)
end sub
sub acceptSearchPage(kind as String, data as Dynamic)
    lane = Mid(kind,7)
    page = searchPage(lane)
    if page = invalid then return
    values = catalogItems(data)
    seen = {}
    for each item in page.items
        seen[item.Id.Replace("-","").LCase()] = true
    end for
    for each item in values
        if item.Id <> invalid and item.Name <> invalid
            key = item.Id.Replace("-","").LCase()
            if not seen.DoesExist(key)
                seen[key] = true
                page.items.Push(item)
            end if
        end if
    end for
    page.next = page.next + values.Count()
    page.total = page.next
    if Type(data) = "roAssociativeArray"
        if values.Count() > 0 and data.TotalRecordCount <> invalid then page.total = data.TotalRecordCount
    end if
    page.busy = false
    searchMenu()
    m.status.text = "Refinements search your account's server metadata."
end sub
sub searchFailed(kind as String)
    page = searchPage(Mid(kind,7))
    if page <> invalid then page.busy = false
    searchMenu()
    m.status.text = "Some search sources could not refresh."
end sub
sub showFacet(field as String)
    if m.facetCache.DoesExist(field)
        renderFacet(field,m.facetCache[field])
        return
    end if
    rows("Refine search",["Loading server metadata"],[{kind:"waitAccount"}])
    if field = "studioId"
        api("facetStudio", "/Studios?UserId=" + m.userId + "&Limit=200", "GET", invalid)
    else if field = "tag"
        api("facetTag", "/Items/Filters2?UserId=" + m.userId, "GET", invalid)
    else
        api("facetCollection", "/Items?UserId=" + m.userId + "&Recursive=true&IncludeItemTypes=BoxSet&Limit=200", "GET", invalid)
    end if
end sub
sub acceptFacet(kind as String, data as Dynamic)
    options = []
    if kind = "facetTag"
        field = "tag"
        if data <> invalid and data.Tags <> invalid
            for each tag in data.Tags
                if options.Count() < 512 then options.Push({id:tag,name:tag})
            end for
        end if
    else
        field = "collectionId"
        if kind = "facetStudio" then field = "studioId"
        for each item in catalogItems(data)
            if item.Id <> invalid and item.Name <> invalid and options.Count() < 200 then options.Push({id:item.Id,name:item.Name})
        end for
    end if
    m.facetCache[field] = options
    renderFacet(field,options)
end sub
sub renderFacet(field as String, options as Object)
    labels = ["All"]
    actions = [{kind:"setFacet",field:field,value:"",name:"All"}]
    for each option in options
        labels.Push(option.name)
        actions.Push({kind:"setFacet",field:field,value:option.id,name:option.name})
    end for
    title = "Collection"
    if field = "studioId" then title = "Studio"
    if field = "tag" then title = "Tag"
    rows(title,labels,actions)
    m.status.text = "Groups and service tags come from your server. Open Collections for additional groups."
end sub
sub acceptSports(data as Dynamic)
    labels = []
    actions = []
    if m.requestKind = "sportsGuide"
        for each item in catalogItems(data)
            if item.Name <> invalid
                labels.Push(item.Name)
                actions.Push({kind:"detail",item:item})
            end if
        end for
    else if data <> invalid and data.games <> invalid and data.local_broadcasts <> invalid
        games = []
        for each game in data.games
            games.Push(game)
        end for
        for each game in data.local_broadcasts
            games.Push(game)
        end for
        for each game in games
            title = "Sports broadcast"
            if game.title <> invalid then title = game.title
            if game.event <> invalid and game.event.name <> invalid then title = game.event.name
            labels.Push(title)
            if game.channel_id <> invalid
                actions.Push({kind:"detail",item:{Id:game.channel_id,Type:"TvChannel"}})
            else
                actions.Push({kind:"waitAccount"})
            end if
        end for
    end if
    if labels.Count() = 0
        labels.Push("Your server has no selected sports events right now")
        actions.Push({kind:"waitAccount"})
    end if
    rows("Sports",labels,actions)
end sub
