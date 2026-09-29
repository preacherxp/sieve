package shop.inventory;

public record Reservation(String id, String sku, int quantity, boolean confirmed) {
    public Reservation confirm() {
        return new Reservation(id, sku, quantity, true);
    }
}
