package example

fun Cents.format(): String = "%d.%02d".format(value / 100, value % 100)
