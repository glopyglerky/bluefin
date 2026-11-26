sub init()
    m.top.functionName = "execute"
end sub
sub execute()
    request = m.top.request
    port = CreateObject("roMessagePort")
    transfer = CreateObject("roUrlTransfer")
    transfer.SetMessagePort(port)
    transfer.SetCertificatesFile("common:/certs/ca-bundle.crt")
    transfer.EnablePeerVerification(true)
    transfer.EnableHostVerification(true)
    transfer.SetUrl(request.server + request.path)
    transfer.AddHeader("Authorization", request.authorization)
    transfer.AddHeader("Content-Type", "application/json")
    transfer.SetRequest(request.method)
    transfer.SetMinimumTransferRate(512, 15)
    if request.method = "GET"
        started = transfer.AsyncGetToString()
    else
        started = transfer.AsyncPostFromString(request.body)
    end if
    if not started
        m.top.result = {ok:false, status:0, message:"Network request could not start."}
        return
    end if
    event = wait(15000, port)
    if event = invalid
        transfer.AsyncCancel()
        m.top.result = {ok:false, status:0, message:"The server did not respond in time."}
        return
    end if
    code = event.GetResponseCode()
    body = event.GetString()
    if Len(body) > 2097152
        m.top.result = {ok:false, status:code, message:"Server response exceeds the TV limit."}
        return
    end if
    data = invalid
    if Len(body) > 0 then data = ParseJson(body)
    m.top.result = {ok:code >= 200 and code < 300, status:code, data:data}
end sub
