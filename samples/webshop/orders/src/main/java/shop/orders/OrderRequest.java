package shop.orders;

public record OrderRequest(String sku, int quantity, String email) {
    public boolean valid() {
        return sku != null && !sku.isBlank() && quantity > 0 && email != null && email.contains("@");
    }
}
