package shop.notifications;

import java.util.List;
import java.util.concurrent.CopyOnWriteArrayList;
import org.springframework.stereotype.Component;
import reactor.core.publisher.Mono;

/** Stands in for the mail gateway: records what would have been sent. */
@Component
public class Outbox {
    private final List<Email> sent = new CopyOnWriteArrayList<>();

    public Mono<Email> send(Email email) {
        return Mono.fromSupplier(() -> {
            sent.add(email);
            return email;
        });
    }

    public List<Email> sent() {
        return List.copyOf(sent);
    }
}
