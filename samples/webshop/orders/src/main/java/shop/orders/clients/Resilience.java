package shop.orders.clients;

import java.time.Duration;
import reactor.util.retry.Retry;
import reactor.util.retry.RetryBackoffSpec;

/** Shared timeout and retry policy for calls to other services. */
final class Resilience {
    static final Duration TIMEOUT = Duration.ofSeconds(3);

    private Resilience() {
    }

    /** Retries server errors and timeouts, never client errors, which will not change. */
    static RetryBackoffSpec retry() {
        return Retry.backoff(2, Duration.ofMillis(50))
            .filter(error -> !(error instanceof DownstreamException e) || e.status().is5xxServerError())
            .onRetryExhaustedThrow((spec, signal) -> signal.failure());
    }
}
