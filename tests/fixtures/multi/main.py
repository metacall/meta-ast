from lib import helper, Utils


def process():
    result = helper()
    formatted = Utils.format(result)
    return formatted
