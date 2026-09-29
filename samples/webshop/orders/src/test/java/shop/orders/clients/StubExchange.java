package shop.orders.clients;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import org.springframework.http.HttpHeaders;
import org.springframework.http.HttpStatus;
import org.springframework.http.MediaType;
import org.springframework.web.reactive.function.client.ClientResponse;
import org.springframework.web.reactive.function.client.WebClient;
import reactor.core.publisher.Mono;

/** Canned downstream responses keyed by "METHOD path", served in order; the last one repeats. */
public class StubExchange {
    private final Map<String, Deque<ClientResponse>> responses = new ConcurrentHashMap<>();
    private final List<String> calls = new ArrayList<>();

    public StubExchange on(String request, HttpStatus status, String json) {
        responses.computeIfAbsent(request, key -> new ArrayDeque<>()).add(ClientResponse.create(status)
            .header(HttpHeaders.CONTENT_TYPE, MediaType.APPLICATION_JSON_VALUE)
            .body(json)
            .build());
        return this;
    }

    public WebClient.Builder builder() {
        return WebClient.builder().exchangeFunction(request -> {
            String key = request.method().name() + " " + request.url().getPath();
            synchronized (calls) {
                calls.add(key);
            }
            Deque<ClientResponse> queue = responses.get(key);
            if (queue == null) {
                return Mono.just(ClientResponse.create(HttpStatus.NOT_IMPLEMENTED).build());
            }
            synchronized (queue) {
                return Mono.just(queue.size() > 1 ? queue.poll() : queue.peek());
            }
        });
    }

    public long calls(String request) {
        synchronized (calls) {
            return calls.stream().filter(request::equals).count();
        }
    }
}
