package example;

import org.springframework.context.event.EventListener;
import org.springframework.scheduling.annotation.Async;
import org.springframework.stereotype.Component;

/** Invoked by the framework on its own thread; no test refers to it. */
@Component
public class OrderListener {
    private final AuditLog log;

    public OrderListener(AuditLog log) {
        this.log = log;
    }

    @Async
    @EventListener
    public void on(OrderPlaced event) {
        log.add("placed " + event.id());
    }
}
