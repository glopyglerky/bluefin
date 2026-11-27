package org.bluefin.android

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlin.coroutines.coroutineContext

data class CatalogLoadSnapshot(
    val offers: List<CatalogOffer> = emptyList(),
    val sources: List<CatalogSource> = emptyList(),
    val completed: Int = 0,
    val total: Int = 0,
    val running: Boolean = false,
    val stoppedReason: String? = null
)

/** Only an explicit All-regions selection enters this bounded country queue. */
internal suspend fun loadAllCatalogCountries(
    identity: CatalogIdentity,
    countries: List<CatalogCountry>,
    initial: CatalogOfferResponse?,
    lookup: suspend (String) -> CatalogOfferResponse,
    update: suspend (CatalogLoadSnapshot) -> Unit
): CatalogLoadSnapshot = coroutineScope {
    val regions = countries.distinctBy { it.code }
    var snapshot = CatalogLoadSnapshot(initial?.offers.orEmpty(), initial?.sources.orEmpty(),
        total = regions.size, running = regions.isNotEmpty())
    update(snapshot)
    for (batch in regions.chunked(3)) {
        coroutineContext.ensureActive()
        val replies = Channel<Pair<String, CatalogOfferResponse>>(batch.size)
        batch.forEach { region ->
            launch {
                val response = try { lookup(region.code) }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { CatalogOfferResponse.failed(identity, region.code) }
                replies.send(region.code to response)
            }
        }
        repeat(batch.size) {
            val (region, reply) = replies.receive()
            coroutineContext.ensureActive()
            val identityValid = reply.identity.matches(identity) && reply.country == region
            val response = if (identityValid) reply else CatalogOfferResponse(identity, region, emptyList(),
                listOf(CatalogSource.failure("invalidResponse", region)))
            val updatedOffers = if (response.knownResult) {
                snapshot.offers.filterNot { it.country == region } + response.offers
            } else snapshot.offers
            val sources = snapshot.sources.filterNot { it.region == region } +
                response.sources.map { it.copy(region = region) }
            val stopped = snapshot.stoppedReason ?: response.sources.firstOrNull { it.transientFailure }?.let {
                when (it.status) {
                    "rateLimited" -> "The source asked Bluefin to pause. Retry the remaining regions later."
                    "denied" -> "The source declined this lookup. Other reported regions remain visible."
                    else -> "A source became unavailable. Retry the remaining regions later."
                }
            }
            snapshot = snapshot.copy(offers = updatedOffers.distinctBy { it.id }, sources = sources,
                completed = snapshot.completed + 1, stoppedReason = stopped)
            update(snapshot)
        }
        replies.close()
        if (snapshot.stoppedReason != null) break
    }
    snapshot = snapshot.copy(running = false)
    update(snapshot)
    snapshot
}
