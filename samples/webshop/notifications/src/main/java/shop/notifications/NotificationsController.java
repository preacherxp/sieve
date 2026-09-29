package shop.notifications;

import java.util.List;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RestController;

@RestController
public class NotificationsController {
    private final Outbox outbox;

    public NotificationsController(Outbox outbox) {
        this.outbox = outbox;
    }

    @GetMapping("/sent")
    public List<Email> sent() {
        return outbox.sent();
    }
}
