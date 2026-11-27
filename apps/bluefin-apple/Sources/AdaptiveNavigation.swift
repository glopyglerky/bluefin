import SwiftUI
import CoreImage

private struct BluefinShowHomeKey: EnvironmentKey {
    static let defaultValue: (() -> Void)? = nil
}

extension EnvironmentValues {
    var bluefinShowHome: (() -> Void)? {
        get { self[BluefinShowHomeKey.self] }
        set { self[BluefinShowHomeKey.self] = newValue }
    }
}

/// Navigation follows the current scene, including resized and folded windows.
struct BluefinNavigation<Content: View>: View {
    private let content: Content
    #if os(tvOS)
    @State private var path: [MediaDestination] = []
    #endif
    init(@ViewBuilder content: () -> Content) { self.content = content() }
    @ViewBuilder var body: some View {
        if #available(iOS 16, tvOS 16, *) {
            #if os(tvOS)
            TVMediaNavigation(path: $path) { content }
            #else
            NavigationStack { content.bluefinNavigationChrome() }
            #endif
        } else {
            NavigationView { content }.navigationViewStyle(.stack)
        }
    }
}

enum MediaDestination: Hashable {
    case detail(MediaItem)
    case searchResult(MediaItem, query: String, accountContext: UUID)
    case trailers(String)
    case collections
    case collection(title: String, parent: MediaItem?, types: String?)
    case account
    case trailerCatalog
    case channel(MediaItem)
    case channels
    case guide
    case musicCollection(MediaItem, type: String)
    case rights
    #if os(tvOS)
    case description(title: String, text: String)
    #endif

    @ViewBuilder func screen(back: (() -> Void)? = nil) -> some View {
        switch self {
        case .detail(let item): DetailView(item: item, onBack: back)
        case .searchResult(let item, let query, let context):
            HomeSearchDestination(item: item, query: query, accountContext: context, onBack: back)
        case .trailers(let id): TrailersDetailView(itemID: id, onBack: back)
        case .collections: LibrariesView().bluefinBackNavigation(back)
        case .collection(let title, let parent, let types):
            ItemGrid(title: title, parent: parent, includeItemTypes: types).bluefinBackNavigation(back)
        case .account: SettingsView().bluefinBackNavigation(back)
        case .trailerCatalog: TrailersView(onBack: back)
        case .channel(let channel): LiveChannelView(channel: channel).bluefinBackNavigation(back)
        case .channels: LiveTVView().bluefinBackNavigation(back)
        case .guide: LiveGuideView().bluefinBackNavigation(back)
        case .musicCollection(let item, let type):
            MusicCollection(title: item.title, type: type, parent: item).bluefinBackNavigation(back)
        case .rights: AuthorizedUseView().bluefinBackNavigation(back)
        #if os(tvOS)
        case .description(let title, let text): TVDescriptionScreen(title: title, text: text).bluefinBackNavigation(back)
        #endif
        }
    }
}

struct MediaNavigationLink<Label: View>: View {
    let destination: MediaDestination
    @ViewBuilder let label: () -> Label

    var body: some View {
        #if os(tvOS)
        if #available(tvOS 16, *) {
            NavigationLink(value: destination, label: label)
        } else {
            NavigationLink(destination: destination.screen(), label: label)
        }
        #else
        NavigationLink(destination: destination.screen(), label: label)
        #endif
    }
}

#if os(tvOS)
private struct TVDescriptionScreen: View {
    let title: String
    let text: String
    @Namespace private var readingFocus

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 24) {
                Text(title).font(.largeTitle.bold()).accessibilityAddTraits(.isHeader)
                ForEach(Array(Self.blocks(text).enumerated()), id: \.offset) { index, block in
                    Text(block)
                        .font(.body).lineSpacing(8)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.vertical, 10)
                        .focusable()
                        .prefersDefaultFocus(index == 0, in: readingFocus)
                }
            }
            .frame(maxWidth: 1160).frame(maxWidth: .infinity).padding(44)
            .focusSection()
        }
        .focusScope(readingFocus)
    }

    static func blocks(_ text: String) -> [String] {
        text.split(separator: "\n").flatMap { paragraph -> [String] in
            var remaining = paragraph
            var result: [String] = []
            while !remaining.isEmpty {
                let part = remaining.prefix(480)
                let end = part.endIndex == remaining.endIndex
                    ? part.endIndex : part.lastIndex(where: \.isWhitespace) ?? part.endIndex
                let boundary = end == remaining.startIndex ? part.endIndex : end
                let block = remaining[..<boundary].trimmingCharacters(in: .whitespaces)
                if !block.isEmpty { result.append(block) }
                remaining = remaining[boundary...].drop(while: \.isWhitespace)
            }
            return result
        }
    }
}

struct TVDetailAction: View {
    let title: String
    let symbol: String
    var prominent = false
    @Environment(\.isFocused) private var focused

    var body: some View {
        Label(title, systemImage: symbol)
            .font(.callout.weight(prominent ? .bold : .medium))
            .foregroundStyle(focused ? BluefinStyle.onPrimary : BluefinStyle.text)
            .padding(.horizontal, 24).padding(.vertical, 14)
            .background(focused ? BluefinStyle.primaryFill : BluefinStyle.secondaryBackground.opacity(0.95), in: Capsule())
            .overlay { Capsule().strokeBorder(BluefinStyle.controlBorder.opacity(focused ? 0 : 1), lineWidth: 1) }
    }
}

@available(tvOS 16, *)
struct TVMediaNavigation<Content: View>: View {
    @Binding var path: [MediaDestination]
    @ViewBuilder let content: () -> Content

    var body: some View {
        NavigationStack(path: $path) {
            content()
                .navigationDestination(for: MediaDestination.self) { destination in
                    destination.screen {
                        guard path.last == destination else { return }
                        path.removeLast()
                    }
                }
        }
    }
}

private struct TVBackControl: ViewModifier {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.bluefinShowHome) private var showHome
    let action: (() -> Void)?
    private func goBack() {
        if let action { action() }
        else { dismiss() }
    }
    func body(content: Content) -> some View {
        content
            .safeAreaInset(edge: .top, spacing: 0) {
                HStack {
                    TVBackButton(action: goBack)
                        .frame(width: 154, height: 58)
                    if let showHome {
                        Button(action: showHome) { Label("Home", systemImage: "house") }
                            .buttonStyle(.bordered)
                            .accessibilityIdentifier("bluefinHome")
                    }
                    Spacer()
                }.padding(.horizontal, 32).padding(.vertical, 10)
                    .background(BluefinStyle.background)
            }
            .onExitCommand(perform: goBack)
    }
}

private struct TVBackButton: UIViewRepresentable {
    let action: () -> Void

    final class Coordinator {
        var action: () -> Void
        init(action: @escaping () -> Void) { self.action = action }
    }

    func makeCoordinator() -> Coordinator { Coordinator(action: action) }

    func makeUIView(context: Context) -> UIButton {
        var style = UIButton.Configuration.gray()
        style.title = "Back"
        style.image = UIImage(systemName: "chevron.left")
        style.imagePadding = 8
        style.cornerStyle = .capsule
        let button = UIButton(configuration: style)
        button.accessibilityIdentifier = "bluefinBack"
        let coordinator = context.coordinator
        button.addAction(UIAction { [weak coordinator] _ in coordinator?.action() }, for: .primaryActionTriggered)
        return button
    }

    func updateUIView(_ button: UIButton, context: Context) { context.coordinator.action = action }
}
#endif

private struct BluefinHomeControl: ViewModifier {
    @Environment(\.bluefinShowHome) private var showHome

    func body(content: Content) -> some View {
        content.toolbar {
            if let showHome {
                ToolbarItem(placement: .primaryAction) {
                    Button(action: showHome) { Label("Home", systemImage: "house") }
                        .accessibilityIdentifier("bluefinHome")
                }
            }
        }
    }
}

extension View {
    @ViewBuilder func bluefinNavigationChrome() -> some View {
        #if os(iOS)
        if #available(iOS 16, *) {
            self.toolbarBackground(BluefinStyle.background, for: .navigationBar)
                .toolbarBackground(.visible, for: .navigationBar)
        } else { self }
        #else
        self
        #endif
    }

    @ViewBuilder func bluefinBackNavigation(_ action: (() -> Void)? = nil) -> some View {
        #if os(tvOS)
        modifier(TVBackControl(action: action))
        #else
        modifier(BluefinHomeControl())
        #endif
    }
}

struct TVWelcomeView: View {
    @EnvironmentObject private var store: LibraryStore
    @State private var server = ""
    @State private var username = ""
    @State private var password = ""
    @State private var allowHTTP = false
    @State private var connectedServer: PublicInfo?
    @State private var accountOptions = AccountOptionsDiscovery()
    @State private var optionsRequest: UUID?
    @State private var checkingOptions = false
    @State private var mode: Mode?
    @State private var passkey = false
    @State private var passwordConfirmation = ""
    @State private var quickConnect = false
    @State private var meshPairing = false
    @State private var publicSearch = false
    @State private var working = false
    @State private var problem: String?
    @FocusState private var focus: Field?
    @Namespace private var signInFocus

    private enum Field: Hashable { case server, continueSetup, passwordOption, username, password, confirmPassword, signIn }
    private enum Mode { case signIn, create }
    private var maximumWidth: CGFloat {
        #if os(tvOS)
        680
        #else
        520
        #endif
    }
    private var inset: CGFloat {
        #if os(tvOS)
        80
        #else
        48
        #endif
    }
    private var address: String {
        let input = server.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !input.isEmpty, !input.contains("://") else { return input }
        return "https://" + input
    }
    private var localHTTP: Bool { address.lowercased().hasPrefix("http://") }

    var body: some View {
        BluefinNavigation {
            GeometryReader { geometry in
                ScrollView(.vertical, showsIndicators: false) {
                    VStack(spacing: 30) {
                        wordmark
                        VStack(spacing: 24) {
                            if let connectedServer { signInOptions(connectedServer) }
                            else {
                                Button { publicSearch = true } label: {
                                    AccountActionLabel(title: "Search media")
                                }.accountButtonStyle().accessibilityIdentifier("publicTVSearch")
                                Button { meshPairing = true } label: {
                                    AccountActionLabel(title: "Connect with QR or invitation")
                                }.accountButtonStyle().accessibilityIdentifier("meshPairing")
                                #if !os(tvOS)
                                Button { store.deviceMusicVisible = true } label: {
                                    AccountActionLabel(title: "Music on this device")
                                }.accountButtonStyle().accessibilityIdentifier("ownedMusicOpen")
                                #endif
                                connectServer
                            }
                            if let problem {
                                Label(problem, systemImage: "exclamationmark.circle")
                                    .font(.callout).foregroundStyle(.secondary)
                                    .multilineTextAlignment(.center)
                                    .fixedSize(horizontal: false, vertical: true)
                                    .accessibilityIdentifier("signInError")
                            }
                        }.accountFocusSection()

                        if connectedServer == nil {
                            HStack(spacing: 24) {
                                Button { store.explore() } label: { AccountActionLabel(title: "Sample library") }
                                    .accountButtonStyle().accessibilityIdentifier("explorePreview")
                                MediaNavigationLink(destination: .rights) { AccountActionLabel(title: "Libraries and rights") }
                                    .accountButtonStyle()
                            }.accountFocusSection()
                        }
                    }
                    .frame(width: min(maximumWidth, max(1, geometry.size.width - inset)))
                    .padding(.vertical, 34)
                    .frame(minHeight: geometry.size.height)
                    .frame(maxWidth: .infinity)
                }
            }
            .background(BluefinBackground())
            .accountFocusScope(signInFocus)
        }
        .sheet(isPresented: $meshPairing) { MeshPairingView().environmentObject(store) }
        .sheet(isPresented: $publicSearch) { PublicTitleSearchView() }
        .sheet(isPresented: $quickConnect) {
            QuickConnectView(server: address, allowHttp: localHTTP && allowHTTP)
                .environmentObject(store)
        }
        .sheet(isPresented: $passkey) {
            PasskeySignInView(server: address).environmentObject(store)
        }
        .task(id: optionsRequest) {
            guard let request = optionsRequest else { return }
            await refreshOptions(request)
        }
    }

    private var wordmark: some View {
        HStack(spacing: 16) {
            BluefinMark().frame(width: 38, height: 38)
            Text("Bluefin").font(.system(size: 56, weight: .bold, design: .rounded))
        }.accessibilityAddTraits(.isHeader)
    }

    private var connectServer: some View {
        VStack(spacing: 18) {
            Text("Connect to your server").font(.title2.weight(.semibold))
            TextField("https://media.example.com", text: $server)
                .bluefinAddressInput().textContentType(.URL).focused($focus, equals: .server)
                .accountDefaultFocus(signInFocus)
                .onSubmit { focus = .continueSetup }
                .accessibilityLabel("Server address").accessibilityIdentifier("serverAddress")
            if localHTTP {
                Toggle("Allow unencrypted local connection", isOn: $allowHTTP)
            }
            Button { Task { await checkServer() } } label: {
                AccountActionLabel(title: working ? "Connecting…" : "Continue", primary: true)
            }
            .accountButtonStyle(primary: true).focused($focus, equals: .continueSetup)
            .disabled(working || address.isEmpty || (localHTTP && !allowHTTP))
            .accessibilityIdentifier("checkTVServer")
        }
    }

    private func signInOptions(_ serverInfo: PublicInfo) -> some View {
        VStack(spacing: 18) {
            Text(serverInfo.ServerName ?? "Sign in").font(.title2.weight(.semibold)).lineLimit(2)
                .accessibilityAddTraits(.isHeader)
            if let mode {
                credentialForm(mode)
                Button {
                    choose(nil)
                    focus = .passwordOption
                } label: { AccountActionLabel(title: "Back to sign-in options") }
                    .accountButtonStyle().disabled(working)
            } else {
                Button { choose(.signIn) } label: {
                    AccountActionLabel(title: "Sign in with password", primary: true)
                }
                .accountButtonStyle(primary: true)
                .focused($focus, equals: .passwordOption)
                .accountDefaultFocus(signInFocus)
                .accessibilityIdentifier("choosePasswordSignIn")
                if accountOptions.choices.passkeys && !localHTTP {
                    Button { passkey = true } label: { AccountActionLabel(title: "Sign in with a passkey") }
                        .accountButtonStyle().accessibilityIdentifier("choosePasskey")
                }
                if accountOptions.choices.quickConnect {
                    Button { quickConnect = true } label: { AccountActionLabel(title: "Quick Connect") }
                        .accountButtonStyle().accessibilityIdentifier("chooseQuickConnect")
                }
                if accountOptions.choices.allowSignups {
                    Button { choose(.create) } label: { AccountActionLabel(title: "Create account") }
                        .accountButtonStyle().accessibilityIdentifier("chooseCreateAccount")
                }
                if checkingOptions {
                    ProgressView("Checking sign-in options").font(.footnote)
                } else if accountOptions.discoveryFailed {
                    Label("Couldn't check account creation and passkey availability.", systemImage: "exclamationmark.circle")
                        .font(.footnote).foregroundStyle(.secondary).multilineTextAlignment(.center)
                    Button(action: checkAccountOptions) { AccountActionLabel(title: "Retry sign-in options") }
                        .accountButtonStyle()
                }
            }
            Button {
                connectedServer = nil
                optionsRequest = nil
                checkingOptions = false
                accountOptions = AccountOptionsDiscovery()
                username = ""
                choose(nil)
                focus = .server
            } label: { AccountActionLabel(title: "Change server") }
                .accountButtonStyle().disabled(working)
        }
    }

    @ViewBuilder private func credentialForm(_ mode: Mode) -> some View {
        let creating = mode == .create
        TextField("Username", text: $username)
            .bluefinAddressInput().textContentType(.username).focused($focus, equals: .username)
            .onSubmit { focus = .password }.accessibilityIdentifier("username")
        SecureField("Password", text: $password)
            .textContentType(creating ? .newPassword : .password).focused($focus, equals: .password)
            .onSubmit { focus = creating ? .confirmPassword : .signIn }.accessibilityIdentifier("password")
        if creating {
            SecureField("Confirm password", text: $passwordConfirmation)
                .textContentType(.newPassword).focused($focus, equals: .confirmPassword)
                .onSubmit { focus = .signIn }
            Text("Use at least 15 characters. Your server administrator can grant access to libraries.")
                .font(.footnote).foregroundStyle(.secondary).multilineTextAlignment(.center)
        }
        Button { Task { await signIn() } } label: {
            AccountActionLabel(title: working ? "Connecting…" : creating ? "Create account" : "Sign in", primary: true)
        }
        .accountButtonStyle(primary: true).focused($focus, equals: .signIn)
        .disabled(working || username.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ||
            (creating && (password.count < 15 || password != passwordConfirmation)))
        .accessibilityIdentifier(creating ? "createAccount" : "connectServer")
    }

    private func choose(_ action: Mode?) {
        mode = action
        password = ""
        passwordConfirmation = ""
        problem = nil
        if action != nil { focus = .username }
    }

    private func checkAccountOptions() {
        checkingOptions = true
        optionsRequest = UUID()
    }

    private func refreshOptions(_ request: UUID) async {
        let http = AccountHTTP(server: address, device: store.deviceId, allowHttp: localHTTP && allowHTTP)
        let result = await http.discoverOptions()
        guard !Task.isCancelled, optionsRequest == request, connectedServer != nil else { return }
        accountOptions = result
        checkingOptions = false
        if mode == .create, !result.choices.allowSignups {
            let issue = problem
            choose(nil)
            problem = issue
            focus = .passwordOption
        }
    }

    private func checkServer() async {
        guard !working, !address.isEmpty else { return }
        working = true; problem = nil
        defer { working = false }
        let requested = address
        let allowed = localHTTP && allowHTTP
        do {
            let info: PublicInfo = try await RustBridge.call("info", fields: [
                "server": requested, "allowHttp": allowed, "deviceId": store.deviceId
            ])
            guard requested == address, allowed == (localHTTP && allowHTTP) else { return }
            guard info.Id != nil || info.ServerName != nil else {
                throw BluefinError.message("This address did not return media server information.")
            }
            connectedServer = info
            focus = .passwordOption
            checkAccountOptions()
        } catch { if requested == address { problem = error.localizedDescription } }
    }

    private func signIn() async {
        guard !working, connectedServer != nil, let mode else { return }
        let user = username.trimmingCharacters(in: .whitespacesAndNewlines)
        let permittedHTTP = localHTTP && allowHTTP
        guard !user.isEmpty else { return }
        if mode == .create {
            guard accountOptions.choices.allowSignups, password.count >= 15, password == passwordConfirmation else { return }
        }
        working = true; problem = nil; focus = nil
        defer { working = false; password = ""; passwordConfirmation = "" }
        do {
            if mode == .create {
                let http = AccountHTTP(server: address, device: store.deviceId, allowHttp: permittedHTTP)
                try await http.write("Bluefin/Accounts/Create", body: ["username": user, "password": password])
                // If login fails after creation, retry sign-in without repeating the write.
                self.mode = .signIn
            }
            try await store.signIn(server: address, username: user, password: password, allowHttp: permittedHTTP)
        } catch {
            let message = error.localizedDescription
            if self.mode == .create { checkAccountOptions() }
            problem = message
            if self.mode != nil { focus = .password }
        }
    }
}

private struct AccountActionLabel: View {
    let title: String
    var primary = false
    #if os(tvOS)
    @Environment(\.isFocused) private var focused
    @Environment(\.isEnabled) private var enabled
    #endif
    var body: some View {
        #if os(tvOS)
        Text(title)
            .font(.callout.weight(primary ? .semibold : .medium))
            .foregroundStyle(focused && enabled ? BluefinStyle.onPrimary : BluefinStyle.text.opacity(enabled ? 1 : 0.55))
            .padding(.horizontal, 24).padding(.vertical, 16)
            .frame(maxWidth: primary ? .infinity : nil)
            .background(focused && enabled ? BluefinStyle.primaryFill : (primary && enabled ? BluefinStyle.hover : BluefinStyle.secondaryBackground),
                        in: Capsule())
        #else
        Text(title).frame(maxWidth: primary ? .infinity : nil)
        #endif
    }
}

private extension View {
    @ViewBuilder func accountButtonStyle(primary: Bool = false) -> some View {
        #if os(tvOS)
        buttonStyle(.plain)
        #else
        if primary { buttonStyle(.borderedProminent).tint(BluefinStyle.primaryFill).foregroundStyle(BluefinStyle.onPrimary) }
        else { buttonStyle(.bordered) }
        #endif
    }

    @ViewBuilder func accountFocusSection() -> some View {
        #if os(tvOS)
        focusSection()
        #else
        self
        #endif
    }

    @ViewBuilder func accountFocusScope(_ scope: Namespace.ID) -> some View {
        #if os(tvOS)
        focusScope(scope)
        #else
        self
        #endif
    }

    @ViewBuilder func accountDefaultFocus(_ scope: Namespace.ID) -> some View {
        #if os(tvOS)
        prefersDefaultFocus(true, in: scope)
        #else
        self
        #endif
    }
}

struct AccountOptions: Decodable, Sendable {
    var allowSignups = false
    var quickConnect = false
    var passkeys = false
}

struct AccountOptionsDiscovery: Sendable {
    var choices = AccountOptions()
    var discoveryFailed = false
}

struct AccountHTTP: Sendable {
    let server: String
    let device: String
    let allowHttp: Bool

    private struct StatusFailure: LocalizedError {
        let code: Int
        var errorDescription: String? { "Server returned HTTP \(code)." }
    }

    private final class StopRedirects: NSObject, URLSessionTaskDelegate {
        func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                        newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
            completionHandler(nil)
        }
    }

    private static let client: URLSession = {
        let settings = URLSessionConfiguration.ephemeral
        settings.httpShouldSetCookies = false
        settings.urlCache = nil
        settings.timeoutIntervalForRequest = 12
        settings.timeoutIntervalForResource = 18
        return URLSession(configuration: settings, delegate: StopRedirects(), delegateQueue: nil)
    }()

    func get<T: Decodable>(_ endpoint: String) async throws -> T {
        try JSONDecoder().decode(T.self, from: await request(endpoint, body: nil))
    }

    func discoverOptions() async -> AccountOptionsDiscovery {
        let discoveryFailed: Bool
        do {
            let options: AccountOptions = try await get("Bluefin/Accounts/Options")
            return AccountOptionsDiscovery(choices: options)
        } catch let failure as StatusFailure where failure.code == 404 || failure.code == 405 {
            discoveryFailed = false
        } catch {
            discoveryFailed = true
        }
        guard !Task.isCancelled else { return AccountOptionsDiscovery() }
        let legacy: Bool? = try? await get("QuickConnect/Enabled")
        return AccountOptionsDiscovery(choices: AccountOptions(quickConnect: legacy == true),
                                       discoveryFailed: discoveryFailed)
    }

    func post<T: Decodable>(_ endpoint: String, body: some Encodable) async throws -> T {
        try JSONDecoder().decode(T.self, from: await request(endpoint, body: JSONEncoder().encode(body)))
    }

    func write(_ endpoint: String, body: some Encodable) async throws {
        _ = try await request(endpoint, body: JSONEncoder().encode(body))
    }

    private func request(_ endpoint: String, body: Data?) async throws -> Data {
        guard let parts = URLComponents(string: server),
              let scheme = parts.scheme?.lowercased(),
              (scheme == "https" || (allowHttp && scheme == "http")),
              parts.host != nil, parts.user == nil, parts.password == nil,
              parts.query == nil, parts.fragment == nil, let base = parts.url else {
            throw BluefinError.message("Enter a valid server address.")
        }
        let url = endpoint.split(separator: "/").reduce(base) { $0.appendingPathComponent(String($1)) }
        #if os(tvOS)
        let platform = "Apple TV"
        #elseif os(visionOS)
        let platform = "Apple Vision Pro"
        #elseif targetEnvironment(macCatalyst)
        let platform = "Mac"
        #else
        let platform = "iOS"
        #endif
        var request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData)
        request.setValue("MediaBrowser Client=\"Bluefin\", Device=\"\(platform)\", DeviceId=\"\(device)\", Version=\"0.1.0\"", forHTTPHeaderField: "Authorization")
        if let body {
            request.httpMethod = "POST"
            request.httpBody = body
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }
        let (data, response) = try await Self.client.data(for: request)
        guard let status = (response as? HTTPURLResponse)?.statusCode else {
            throw BluefinError.message("The server did not return an HTTP response.")
        }
        guard (200..<300).contains(status) else {
            throw StatusFailure(code: status)
        }
        guard data.count <= 65536 else { throw BluefinError.message("Server response too large.") }
        return data
    }
}

struct PasskeySignInView: View {
    let server: String
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    @State private var request: Challenge?
    @State private var qr: CGImage?
    @State private var problem: String?
    @State private var attempt = UUID()

    private struct Challenge: Decodable, Sendable {
        struct Proof: Codable, Sendable {
            struct Pairing: Codable, Sendable {
                let challenge: String
                let secret: String
            }
            let ticket: String
            let pairing: Pairing
        }
        let url: URL
        let code: String
        let proof: Proof
    }

    private struct Exchange: Decodable, Sendable {
        let pending: Bool?
        let User: LoginResult.User?
        let AccessToken: String?
        let ServerId: String?
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 20) {
                Text("Sign in with a passkey").font(.title2.bold())
                if let request {
                    #if os(tvOS)
                    Text("Scan with your phone, then check the code.").foregroundStyle(.secondary)
                    #else
                    Link("Open on this device", destination: request.url).buttonStyle(.borderedProminent).tint(BluefinStyle.primaryFill).foregroundStyle(BluefinStyle.onPrimary)
                    Text("Or scan with another device.").foregroundStyle(.secondary)
                    #endif
                    if let qr {
                        Image(decorative: qr, scale: 1)
                            .interpolation(.none).resizable().scaledToFit()
                            .frame(width: 210, height: 210).padding(12).background(.white)
                            .accessibilityHidden(true)
                    }
                    Text(request.code).font(.system(.title2, design: .monospaced).bold()).tracking(5)
                        .accessibilityIdentifier("passkeyConfirmationCode")
                }
                if let problem {
                    Text(problem).multilineTextAlignment(.center).foregroundStyle(.secondary)
                    Button("Try again") { attempt = UUID() }
                } else {
                    ProgressView(request == nil ? "Preparing passkey sign-in" : "Waiting for approval")
                }
                Button("Cancel") { dismiss() }
            }
            .padding(32).frame(maxWidth: 620).frame(maxWidth: .infinity)
        }
        .task(id: attempt) { await signIn() }
    }

    private func signIn() async {
        request = nil; qr = nil; problem = nil
        let http = AccountHTTP(server: server, device: store.deviceId, allowHttp: false)
        var proof: Challenge.Proof?
        var finished = false
        defer {
            if !finished, let proof {
                Task.detached {
                    try? await http.write("Bluefin/Accounts/Passkey/Cancel", body: proof)
                }
            }
        }
        do {
            let challenge: Challenge = try await http.post("Bluefin/Accounts/Passkey/Initiate", body: [String: String]())
            proof = challenge.proof
            guard let origin = URL(string: server),
                  let expected = URLComponents(url: origin.appendingPathComponent("Bluefin").appendingPathComponent("Accounts").appendingPathComponent("Passkey"), resolvingAgainstBaseURL: false),
                  let actual = URLComponents(url: challenge.url, resolvingAgainstBaseURL: false),
                  actual.scheme == "https", actual.host == expected.host, actual.port == expected.port,
                  actual.path == expected.path, actual.user == nil, actual.password == nil,
                  actual.query == nil, actual.fragment?.count == 32 else {
                throw BluefinError.message("The server sent an unexpected passkey link.")
            }
            let filter = CIFilter(name: "CIQRCodeGenerator", parameters: ["inputMessage": Data(challenge.url.absoluteString.utf8)])
            if let image = filter?.outputImage {
                qr = CIContext().createCGImage(image, from: image.extent)
            }
            request = challenge
            for _ in 0..<99 {
                try await Task.sleep(nanoseconds: 3_000_000_000)
                let response: Exchange = try await http.post("Bluefin/Accounts/Passkey/Exchange", body: challenge.proof)
                if response.pending == true { continue }
                guard let user = response.User, let token = response.AccessToken else {
                    throw BluefinError.message("The server returned an incomplete sign-in.")
                }
                try await store.acceptLogin(LoginResult(User: user, AccessToken: token, ServerId: response.ServerId),
                                            server: server, allowHttp: false)
                finished = true
                dismiss()
                return
            }
            problem = "This request expired. Start another to sign in."
        } catch is CancellationError {} catch {
            problem = error.localizedDescription
        }
    }
}

#if os(tvOS)
struct TVHomeView: View {
    @EnvironmentObject private var store: LibraryStore
    @Binding var shelfItem: MediaItem?
    @ObservedObject var sports: SportsStore
    var openSearch: () -> Void = {}
    var openCollections: () -> Void = {}
    var openAccount: () -> Void = {}

    private var shelfIsPresented: Binding<Bool> {
        Binding(get: { shelfItem != nil }, set: { if !$0 { shelfItem = nil } })
    }

    @ViewBuilder var body: some View {
        if #available(tvOS 16, *) {
            home.navigationDestination(isPresented: shelfIsPresented) {
                if let shelfItem { DetailView(item: shelfItem) }
            }
        } else {
            home.background {
                NavigationLink(isActive: shelfIsPresented) {
                    if let shelfItem { DetailView(item: shelfItem) }
                } label: { EmptyView() }
            }
        }
    }

    private var home: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 42) {
                HStack(spacing: 22) {
                    Text("Home")
                        .font(.largeTitle.bold())
                        .accessibilityAddTraits(.isHeader)
                        .accessibilityIdentifier("homeHeading")
                    Spacer()
                    Button(action: openSearch) { Label("Search", systemImage: "magnifyingglass") }.accessibilityIdentifier("openSearch")
                    Button(action: openCollections) { Label("Collections", systemImage: "square.stack") }.accessibilityIdentifier("openCollections")
                    Button(action: openAccount) { Label("Account", systemImage: "person.crop.circle") }.accessibilityIdentifier("openAccount")
                }.buttonStyle(.bordered).padding(.horizontal, 32).focusSection()
                if store.preview {
                    Label("Sample library", systemImage: "sparkles")
                        .font(.caption).foregroundStyle(.secondary).padding(.horizontal, 32)
                }
                if let featured = store.home.featured { TVFeaturedItem(item: featured) }
                if (store.loading || (!store.initialHomeLoaded && !store.preview)) && store.home.latest.isEmpty && store.home.resume.isEmpty && store.home.nextUp.isEmpty {
                    ProgressView("Loading your library").padding(.horizontal, 32)
                }
                if !store.home.resume.isEmpty { MediaShelf(title: "Continue watching", items: store.home.resume) }
                if !store.home.nextUp.isEmpty { MediaShelf(title: "Up next", items: store.home.nextUp) }
                ForEach(store.home.recommendations) { shelf in
                    if !shelf.items.isEmpty { MediaShelf(title: shelf.title, items: shelf.items) }
                }
                let movies = store.home.latest.filter { $0.Type == "Movie" }
                let shows = store.home.latest.filter { ["Series", "Season", "Episode"].contains($0.Type ?? "") }
                if !movies.isEmpty { MediaShelf(title: store.homeFromCatalog ? "Movies" : "Recently added movies", items: movies) }
                if !shows.isEmpty { MediaShelf(title: store.homeFromCatalog ? "TV Shows" : "Recently added shows", items: shows) }
                if let issue = store.libraryProblem {
                    VStack(alignment: .leading, spacing: 12) {
                        Label(issue, systemImage: "arrow.clockwise").font(.callout).foregroundStyle(.secondary).lineLimit(2)
                        Button("Refresh library") { Task { await store.refresh() } }.disabled(store.loading)
                    }.padding(.horizontal, 32)
                }
                if !store.preview {
                    if sports.feed == nil {
                        NavigationLink(destination: SportsDashboard(sports: sports).bluefinBackNavigation()) {
                            Label("Sports guide", systemImage: "sportscourt")
                        }.buttonStyle(.bordered).padding(.horizontal, 32)
                    }
                    SportsHomeSection(sports: sports)
                }
            }.padding(.vertical, 28)
        }
        .background(BluefinBackground())
    }
}

private struct TVFeaturedItem: View {
    @EnvironmentObject private var store: LibraryStore
    let item: MediaItem

    var body: some View {
        ZStack(alignment: .bottomLeading) {
            Artwork(item: item, backdrop: true, thumbnailWidth: 1440)
            LinearGradient(colors: [.black.opacity(0.88), .black.opacity(0.16), .clear],
                           startPoint: .bottomLeading, endPoint: .topTrailing)
            VStack(alignment: .leading, spacing: 14) {
                Text("FEATURED").font(.caption.weight(.semibold)).tracking(1.5)
                Text(item.title).font(.largeTitle.bold()).lineLimit(2)
                if !item.subtitle.isEmpty { Text(item.subtitle).font(.callout).foregroundStyle(.white.opacity(0.8)) }
                HStack(spacing: 20) {
                    if !item.isContainer {
                        Button { Task { await store.play(item) } } label: {
                            Label(item.resumeTicks > 0 ? "Resume" : "Play", systemImage: "play.fill")
                        }.buttonStyle(.borderedProminent).tint(BluefinStyle.primaryFill).foregroundStyle(BluefinStyle.onPrimary).disabled(store.startingPlayback)
                    }
                    MediaNavigationLink(destination: .detail(item)) {
                        Text(item.Type == "Series" ? "Episodes" : "Details")
                    }.buttonStyle(.bordered).tint(.white).foregroundStyle(.white)
                }
            }.foregroundStyle(.white).padding(42)
        }.frame(height: 370).clipShape(RoundedRectangle(cornerRadius: 26))
            .padding(.horizontal, 32)
    }
}

enum TVVideoLibraryKind {
    case movies, shows
    var title: String { self == .movies ? "Movies" : "TV Shows" }
    var itemType: String { self == .movies ? "Movie" : "Series" }
    var collectionType: String { self == .movies ? "movies" : "tvshows" }
    var icon: String { self == .movies ? "film" : "tv" }
}

struct TVVideoLibrary: View {
    @EnvironmentObject private var store: LibraryStore
    let kind: TVVideoLibraryKind

    var body: some View {
        let collections = store.libraries.filter { $0.CollectionType == kind.collectionType }
        ItemGrid(title: kind.title, parent: nil, includeItemTypes: kind.itemType)
            .safeAreaInset(edge: .top) {
                if collections.count > 1 {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 18) {
                            ForEach(collections) { collection in
                                MediaNavigationLink(destination: .collection(title: collection.title, parent: collection, types: kind.itemType)) {
                                    Label(collection.title, systemImage: kind.icon)
                                }.buttonStyle(.bordered)
                            }
                        }.padding(.horizontal, 34).padding(.vertical, 10)
                    }.background(BluefinStyle.background).focusSection()
                }
            }
            .background(BluefinBackground())
    }
}

struct TVEpisodeShelf: View {
    @EnvironmentObject private var store: LibraryStore
    let parent: MediaItem
    @State private var entries: [MediaItem] = []
    @State private var failure: String?
    @State private var loaded = false
    @State private var isSeasons = false
    @State private var hasMore = false
    @State private var retry = UUID()

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            if !loaded { ProgressView("Loading episodes") }
            if let failure {
                Text(failure).font(.callout).foregroundStyle(.secondary)
                Button("Try again") { retry = UUID() }
            } else if loaded && entries.isEmpty {
                Text("No episodes are available to this account.").foregroundStyle(.secondary)
            } else {
                if !entries.isEmpty {
                    MediaShelf(title: isSeasons ? "Seasons" : "Episodes", items: entries)
                }
                if hasMore {
                    MediaNavigationLink(destination: .collection(title: parent.title, parent: parent,
                                                                  types: isSeasons ? "Season" : "Episode")) {
                        Text(isSeasons ? "All seasons" : "All episodes")
                    }.buttonStyle(.bordered)
                }
            }
        }.task(id: retry) { await load() }
    }

    private func load() async {
        loaded = false; failure = nil
        defer { if !Task.isCancelled { loaded = true } }
        do {
            if parent.Type == "Series" {
                let seasons = try await store.items(parent: parent, includeItemTypes: "Season")
                try Task.checkCancellation()
                if !seasons.Items.isEmpty { show(seasons, seasons: true); return }
            }
            let episodes = try await store.items(parent: parent, includeItemTypes: "Episode")
            try Task.checkCancellation()
            show(episodes, seasons: false)
        } catch is CancellationError {} catch { failure = error.localizedDescription }
    }

    private func show(_ page: ItemPage, seasons: Bool) {
        isSeasons = seasons
        entries = page.Items.sorted {
            let left = ($0.ParentIndexNumber ?? 0, $0.IndexNumber ?? 0)
            let right = ($1.ParentIndexNumber ?? 0, $1.IndexNumber ?? 0)
            if left != right { return left < right }
            return $0.title.localizedStandardCompare($1.title) == .orderedAscending
        }
        hasMore = (page.TotalRecordCount ?? page.Items.count) > page.Items.count
    }
}
#endif
