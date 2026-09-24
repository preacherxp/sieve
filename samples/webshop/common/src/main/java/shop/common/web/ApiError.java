package shop.common.web;

/** Error body shared by every service. */
public record ApiError(String code, String message) {
}
