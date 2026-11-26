' Original native SceneGraph adapter. Roku cannot execute the Rust/WASM policy.
sub init()
    m.top.backgroundColor = "0x061727FF"
    m.list = m.top.findNode("list")
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
    m.history = []
    m.playbackScreen = invalid
    m.guidePending = false
    if m.token = ""
        rows("Bluefin", ["Connect to server"], [{kind:"connect"}])
    else
        rows("Home", ["Movies", "TV Shows", "Live TV", "Music", "Search", "Queue", "Sign out"], [{kind:"movies"}, {kind:"shows"}, {kind:"guide"}, {kind:"music"}, {kind:"search"}, {kind:"queue"}, {kind:"signout"}])
    end if
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
        loadLibrary("/Items?Recursive=true&IncludeItemTypes=Audio,MusicAlbum&Limit=100&Fields=Overview,MediaStreams",0)
    else if action.kind = "search"
        keyboard("Search your library", "search", false)
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
        transfer = CreateObject("roUrlTransfer")
        query = transfer.Escape(text)
        loadLibrary("/Items?Recursive=true&Limit=100&SearchTerm=" + query + "&IncludeItemTypes=Movie,Series,Episode,Audio,MusicAlbum&Fields=Overview,MediaStreams",0)
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
    request = {kind:kind, server:m.server, path:path, method:method, body:text, authorization:authorization()}
    if m.task <> invalid
        if kind <> "report" and m.requests.Count() < 16 then m.requests.Push(request)
        return
    end if
    startRequest(request)
end sub
sub startRequest(request as Object)
    m.requestKind = request.kind
    m.task = CreateObject("roSGNode", "ApiTask")
    m.task.request = request
    m.task.ObserveField("result", "response")
    m.task.control = "RUN"
end sub
sub drainRequests()
    if m.task = invalid and m.requests.Count() > 0
        request = m.requests.Shift()
        startRequest(request)
    end if
end sub
sub response()
    result = m.task.result
    m.task.UnobserveField("result")
    m.task = invalid
    if not result.ok
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
        if result.status = 401
            m.token = ""
            stopPlayer()
            menu()
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
