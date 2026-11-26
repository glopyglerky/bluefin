package org.bluefin.android

import kotlinx.coroutines.CompletableDeferred

/** Called on the main dispatcher. A stopped screen retains ownership until its
 * preparation/finalizer has actually returned, including after composition ends.
 */
internal class VideoReaders {
    internal class Lease(val stop: () -> Unit) {
        val retired = CompletableDeferred<Unit>()
    }
    private val readers = linkedSetOf<Lease>()
    fun register(stop: () -> Unit): Lease? =
        if (readers.size >= 32) null else Lease(stop).also { readers.add(it) }

    fun finished(lease: Lease?) {
        if (lease != null && readers.remove(lease)) lease.retired.complete(Unit)
    }
    fun failed(lease: Lease?, failure: Throwable) {
        if (lease != null && lease in readers) lease.retired.completeExceptionally(failure)
    }
    suspend fun stopAndJoin() {
        val captured = readers.toList()
        var failure: Throwable? = null
        // Signal every reader before waiting for any individual finalizer.
        captured.forEach { runCatching(it.stop).onFailure { error -> if (failure == null) failure = error } }
        failure?.let { throw it }
        captured.forEach { it.retired.await() }
    }
}

internal val foregroundVideoReaders = VideoReaders()
