package org.bluefin.android

import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class VideoReadersTest {
    @Test fun signOutSignalsEveryCapturedReaderAndWaitsForActualRetirement() = runBlocking {
        val readers = VideoReaders()
        val stopped = mutableListOf<String>()
        val first = readers.register { stopped.add("first") }!!
        val second = readers.register { stopped.add("second") }!!
        val join = launch(start = CoroutineStart.UNDISPATCHED) { readers.stopAndJoin() }
        assertEquals(listOf("first", "second"), stopped)
        assertFalse(join.isCompleted)
        readers.finished(second)
        assertFalse(join.isCompleted)
        readers.finished(first)
        join.join()
        assertTrue(join.isCompleted)
    }
    @Test fun wrongRegistryCannotConfirmAnotherReadersRetirement() = runBlocking {
        val owner = VideoReaders()
        val lease = owner.register {}!!
        VideoReaders().finished(lease)
        assertFalse(lease.retired.isCompleted)
        owner.finished(lease)
        owner.stopAndJoin()
    }
    @Test fun failedStopStillSignalsOtherReadersAndPreservesTheirOwnership() = runBlocking {
        val readers = VideoReaders()
        val first = readers.register { error("synthetic stop failure") }!!
        var otherStopped = false
        val second = readers.register { otherStopped = true }!!
        assertTrue(runCatching { readers.stopAndJoin() }.isFailure)
        assertTrue(otherStopped)
        assertFalse(first.retired.isCompleted)
        assertFalse(second.retired.isCompleted)
        readers.finished(first); readers.finished(second)
        readers.stopAndJoin()
    }
    @Test fun pendingReadersAreBoundedUntilTheirOwnersActuallyFinish() {
        val readers = VideoReaders()
        val leases = (1..32).map { readers.register {}!! }
        assertNull(readers.register {})
        readers.finished(leases.first())
        val replacement = readers.register {}!!
        assertFalse(replacement.retired.isCompleted)
        leases.drop(1).forEach(readers::finished)
        readers.finished(replacement)
    }
}
