package bookstore.notify;

import java.util.ArrayList;
import java.util.List;

public class Notifier {
    private final EmailFormatter formatter = new EmailFormatter();
    private final List<String> outbox = new ArrayList<>();

    public void orderPlaced(String customer, String title, int quantity) {
        outbox.add(formatter.subject(title) + "\n" + formatter.body(customer, title, quantity));
    }

    public List<String> outbox() {
        return List.copyOf(outbox);
    }
}
