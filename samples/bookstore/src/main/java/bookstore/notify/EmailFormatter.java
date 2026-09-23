package bookstore.notify;

public class EmailFormatter {
    public String subject(String title) {
        return "Your order: " + title;
    }

    public String body(String customer, String title, int quantity) {
        return "Hi " + customer + ",\n\nThanks for ordering " + quantity + " x " + title + ".\n";
    }
}
