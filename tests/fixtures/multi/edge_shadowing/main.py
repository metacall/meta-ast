from lib_shadow import helper

def helper():
    return "local helper"

def run():
    return helper()  # should call local helper, not imported
