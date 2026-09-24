package shop.orders.clients;

public record ReservationView(String id, String sku, int quantity, boolean confirmed) {
}
