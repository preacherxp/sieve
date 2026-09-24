package shop.notifications;

import org.springframework.stereotype.Component;
import shop.common.events.OrderPlaced;

@Component
public class EmailRenderer {
    public Email confirmation(OrderPlaced order) {
        String subject = "Order " + order.orderId().substring(0, 8) + " confirmed";
        String body = """
            Thanks for your order!

            %d x %s
            Total: %s
            """.formatted(order.quantity(), order.productName(), order.total().format());
        return new Email(order.email(), subject, body);
    }
}
