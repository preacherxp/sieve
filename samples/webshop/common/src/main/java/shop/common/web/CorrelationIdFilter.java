package shop.common.web;

import java.util.UUID;
import org.springframework.web.server.ServerWebExchange;
import org.springframework.web.server.WebFilter;
import org.springframework.web.server.WebFilterChain;
import reactor.core.publisher.Mono;

/** Propagates the caller's correlation id, or starts a new one, through headers and the Reactor context. */
public class CorrelationIdFilter implements WebFilter {
    @Override
    public Mono<Void> filter(ServerWebExchange exchange, WebFilterChain chain) {
        String incoming = exchange.getRequest().getHeaders().getFirst(CorrelationId.HEADER);
        String id = incoming == null || incoming.isBlank() ? UUID.randomUUID().toString() : incoming;
        exchange.getResponse().getHeaders().set(CorrelationId.HEADER, id);
        return chain.filter(exchange).contextWrite(context -> context.put(CorrelationId.CONTEXT_KEY, id));
    }
}
