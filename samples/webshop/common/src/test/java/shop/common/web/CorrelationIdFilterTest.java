package shop.common.web;

import static org.assertj.core.api.Assertions.assertThat;

import java.util.concurrent.atomic.AtomicReference;
import org.junit.jupiter.api.Test;
import org.springframework.mock.http.server.reactive.MockServerHttpRequest;
import org.springframework.mock.web.server.MockServerWebExchange;
import reactor.core.publisher.Mono;

class CorrelationIdFilterTest {
    private final CorrelationIdFilter filter = new CorrelationIdFilter();

    @Test
    void keepsIncomingIdAndExposesItToTheChain() {
        var exchange = MockServerWebExchange.from(MockServerHttpRequest.get("/").header(CorrelationId.HEADER, "abc"));
        var seen = new AtomicReference<String>();
        filter.filter(exchange, e -> Mono.deferContextual(context -> {
            seen.set(context.get(CorrelationId.CONTEXT_KEY));
            return Mono.empty();
        })).block();
        assertThat(seen).hasValue("abc");
        assertThat(exchange.getResponse().getHeaders().getFirst(CorrelationId.HEADER)).isEqualTo("abc");
    }

    @Test
    void startsNewIdWhenMissing() {
        var exchange = MockServerWebExchange.from(MockServerHttpRequest.get("/"));
        filter.filter(exchange, e -> Mono.empty()).block();
        assertThat(exchange.getResponse().getHeaders().getFirst(CorrelationId.HEADER)).hasSize(36);
    }
}
